//! The first-run flow: pick a network, a nickname, maybe an account and
//! channels, then watch the first connection come up.

use crate::effect::{Effect, Reply, ReqId};
use crate::onboarding::{explain_error, nick_error, parse_channels, presets, ErrorContext, Friendly, NetworkPreset, PresetKind};
use crate::state::App;
use crate::types::{ConnStatus, LineType, SaslMechanism, ServerConfig};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Network,
    Server,
    Nick,
    Account,
    Channels,
    Custom,
    Connect,
}

impl Step {
    pub fn label(self) -> &'static str {
        match self {
            Step::Network => "Network",
            Step::Server | Step::Custom => "Server",
            Step::Nick => "Nickname",
            Step::Account => "Account",
            Step::Channels => "Channels",
            Step::Connect => "Connect",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub raw: String,
    pub sasl: bool,
}

/// One line of the connection progress list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stage {
    pub key: &'static str,
    pub label: String,
}

pub struct WelcomeFlow {
    pub step: Step,
    pub preset: Option<NetworkPreset>,
    pub server: ServerConfig,
    /// Set once saved, so retries update the same server instead of adding another.
    pub saved_id: String,
    pub nick_touched: bool,
    pub use_account: bool,
    pub account_error: String,
    pub server_error: String,
    pub picked: Vec<String>,
    pub extra: String,
    pub saving: bool,
    /// Line ids are sequential, so anything above this belongs to the current attempt.
    baseline: u64,
    /// The backend has picked up this attempt, so a disconnect belongs to it.
    live: bool,
    pub failure: Option<Failure>,
    save_req: Option<ReqId>,
}

impl Default for WelcomeFlow {
    fn default() -> Self {
        WelcomeFlow {
            step: Step::Network,
            preset: None,
            server: default_server(),
            saved_id: String::new(),
            nick_touched: false,
            use_account: false,
            account_error: String::new(),
            server_error: String::new(),
            picked: Vec::new(),
            extra: String::new(),
            saving: false,
            baseline: 0,
            live: false,
            failure: None,
            save_req: None,
        }
    }
}

/// A blank server with the usual defaults (TLS on 6697, autoconnect).
pub fn default_server() -> ServerConfig {
    ServerConfig::default()
}

impl WelcomeFlow {
    pub fn presets() -> Vec<NetworkPreset> {
        presets()
    }

    /// The steps for the chosen network, in order.
    pub fn flow(&self) -> Vec<Step> {
        match &self.preset {
            None => vec![Step::Network],
            Some(p) if p.kind == PresetKind::Custom => vec![Step::Network, Step::Custom, Step::Connect],
            Some(p) if p.kind == PresetKind::Bouncer => vec![Step::Network, Step::Server, Step::Nick, Step::Account, Step::Connect],
            Some(p) => {
                let mut v = vec![Step::Network, Step::Nick];
                if p.sasl == Some(true) {
                    v.push(Step::Account);
                }
                v.extend([Step::Channels, Step::Connect]);
                v
            }
        }
    }

    pub fn index(&self) -> usize {
        self.flow().iter().position(|s| *s == self.step).unwrap_or(0)
    }

    pub fn nick_len(&self) -> usize {
        self.preset.as_ref().and_then(|p| p.nick_len).unwrap_or(30)
    }

    pub fn nick_problem(&self) -> Option<String> {
        nick_error(self.server.nick.trim(), self.nick_len(), self.preset.as_ref().map_or("", |p| p.label))
    }

    /// The network's name as the progress list says it.
    pub fn net_name(&self) -> String {
        match &self.preset {
            Some(p) if p.kind == PresetKind::Network => p.label.to_string(),
            _ if !self.server.name.is_empty() => self.server.name.clone(),
            _ => self.server.host.clone(),
        }
    }

    pub fn next(&mut self) {
        let f = self.flow();
        if let Some(s) = f.get(self.index() + 1) {
            self.step = *s;
        }
    }

    pub fn back(&mut self) {
        let f = self.flow();
        if let Some(s) = self.index().checked_sub(1).and_then(|i| f.get(i)) {
            self.step = *s;
        }
    }
}

impl App {
    /// Pick a network preset by id (`libera`, `soju`, `custom`, ...) and move on.
    pub fn welcome_choose(&mut self, preset_id: &str) {
        let Some(p) = presets().into_iter().find(|p| p.id == preset_id) else { return };
        let keep_nick = self.welcome.server.nick.clone();
        // A failed attempt was already saved; don't leave it behind when switching networks.
        let stale = std::mem::take(&mut self.welcome.saved_id);
        if !stale.is_empty() {
            self.delete_server(&stale);
        }
        let w = &mut self.welcome;
        w.server = ServerConfig {
            name: if p.kind == PresetKind::Network { p.label.to_string() } else { String::new() },
            host: p.host.to_string(),
            port: p.port,
            tls: p.tls,
            nick: keep_nick,
            ..default_server()
        };
        w.use_account = p.kind == PresetKind::Bouncer;
        w.picked = p.channels.iter().map(|c| c.to_string()).collect();
        w.extra.clear();
        w.nick_touched = false;
        w.preset = Some(p);
        w.step = w.flow()[1];
        self.env.touch();
    }

    pub fn welcome_submit_server(&mut self) {
        let w = &mut self.welcome;
        w.server.host = w.server.host.trim().to_string();
        w.server_error = if w.server.host.is_empty() { "Enter the address of your bouncer.".into() } else { String::new() };
        if w.server_error.is_empty() {
            w.next();
        }
        self.env.touch();
    }

    pub fn welcome_submit_nick(&mut self) {
        let w = &mut self.welcome;
        w.nick_touched = true;
        w.server.nick = w.server.nick.trim().to_string();
        if w.nick_problem().is_none() {
            w.next();
        }
        self.env.touch();
    }

    pub fn welcome_submit_account(&mut self) {
        let w = &mut self.welcome;
        w.account_error.clear();
        if w.use_account && w.server.sasl_password.is_empty() {
            w.account_error = "Enter your password, or turn off \u{201c}I have an account\u{201d}.".into();
            self.env.touch();
            return;
        }
        if w.preset.as_ref().is_some_and(|p| p.kind == PresetKind::Network) {
            w.next();
            self.env.touch();
        } else {
            self.welcome_connect(None);
        }
    }

    pub fn welcome_submit_channels(&mut self) {
        self.welcome_connect(None);
    }

    /// Save the server and connect. `from` replaces the server (the custom form submits its own).
    pub fn welcome_connect(&mut self, from: Option<ServerConfig>) {
        let seq = self.env.next_seq();
        let w = &mut self.welcome;
        if let Some(s) = from {
            w.server = s;
        } else if w.preset.as_ref().map(|p| p.kind) != Some(PresetKind::Custom) {
            w.server.sasl = if w.use_account { SaslMechanism::Plain } else { SaslMechanism::None };
            if !w.use_account {
                w.server.sasl_password.clear();
            }
            if w.use_account && w.server.sasl_username.trim().is_empty() {
                w.server.sasl_username = w.server.nick.clone();
            }
            if w.preset.as_ref().is_some_and(|p| p.kind == PresetKind::Network) {
                let text = w.picked.iter().cloned().chain(std::iter::once(w.extra.clone())).collect::<Vec<_>>().join(" ");
                w.server.autojoin = parse_channels(&text);
            }
        }
        if !w.saved_id.is_empty() {
            w.server.id = w.saved_id.clone();
        }
        w.step = Step::Connect;
        w.failure = None;
        w.live = false;
        w.saving = true;
        w.baseline = seq;
        let server = w.server.clone();
        let req = self.save_server(server, true);
        self.welcome.save_req = Some(req);
        self.env.touch();
    }

    /// Send the user back to the step that can fix the problem.
    pub fn welcome_edit_after_failure(&mut self) {
        let w = &mut self.welcome;
        let auth = w.failure.as_ref().is_some_and(|f| {
            let r = f.raw.to_lowercase();
            r.contains("sasl") || r.contains("authenticat")
        });
        w.failure = None;
        let flow = w.flow();
        w.step = if auth && flow.contains(&Step::Account) {
            Step::Account
        } else if flow.contains(&Step::Server) {
            Step::Server
        } else {
            flow[flow.len().saturating_sub(2)]
        };
        self.env.touch();
    }

    /// The failure in plain language, if the attempt failed.
    pub fn welcome_friendly(&self) -> Option<Friendly> {
        let w = &self.welcome;
        w.failure.as_ref().map(|f| explain_error(&f.raw, &ErrorContext { host: &w.server.host, port: w.server.port, tls: w.server.tls, sasl: f.sasl }))
    }

    /// A note when the server gave us another nick than the one we asked for.
    pub fn welcome_nick_note(&self) -> Option<String> {
        let c = self.conns.get(&self.welcome.saved_id)?;
        (c.status == ConnStatus::Connected && !c.nick.is_empty() && c.fold(&c.nick) != c.fold(&self.welcome.server.nick))
            .then(|| format!("\u{201c}{}\u{201d} was taken, so you're \u{201c}{}\u{201d} for now.", self.welcome.server.nick, c.nick))
    }

    /// The server refused the account login during this attempt (it connected without it).
    pub fn welcome_sasl_failed(&self) -> bool {
        let w = &self.welcome;
        self.conns.get(&w.saved_id).is_some_and(|c| sasl_error(c, w.baseline).is_some())
    }

    /// Where the connection attempt stands: `saving`, `connecting`, `registering`, `connected` or `disconnected`.
    pub fn welcome_status(&self) -> &'static str {
        if self.welcome.saving {
            return "saving";
        }
        match self.conns.get(&self.welcome.saved_id).map(|c| c.status) {
            Some(ConnStatus::Registering) => "registering",
            Some(ConnStatus::Connected) => "connected",
            Some(ConnStatus::Disconnected) => "disconnected",
            _ => "connecting",
        }
    }

    /// The progress list and how far along it is (an index into it).
    pub fn welcome_stages(&self) -> (Vec<Stage>, usize) {
        let w = &self.welcome;
        let plain = w.server.sasl == SaslMechanism::Plain;
        let stages = vec![
            Stage { key: "saving", label: if plain { "Saving settings, password to your keychain".into() } else { "Saving settings".into() } },
            Stage { key: "connecting", label: format!("Reaching {}{}", w.server.host, if w.server.tls { " securely" } else { "" }) },
            Stage { key: "registering", label: if plain { "Logging in to your account".into() } else { "Signing on".into() } },
            Stage {
                key: "connected",
                label: if w.preset.as_ref().is_some_and(|p| p.kind == PresetKind::Bouncer) {
                    "Connected to your bouncer".into()
                } else {
                    format!("Connected to {}", w.net_name())
                },
            },
        ];
        let status = match self.welcome_status() {
            "disconnected" => "connecting",
            s => s,
        };
        let at = stages.iter().position(|s| s.key == status).unwrap_or(0);
        (stages, at)
    }

    /// The user is done with the flow (or leaves it after connecting).
    pub fn welcome_done(&mut self) {
        self.onboarding = false;
        self.env.touch();
    }
}

/// The first `SASL:` error the server buffer got since the attempt began (line ids only grow).
fn sasl_error(c: &crate::conn::Conn, baseline: u64) -> Option<String> {
    c.server.lines().iter().find(|l| l.kind == LineType::Error && l.text.starts_with("SASL:") && l.id > baseline).map(|l| l.text.clone())
}

/// Follow the save and the connection: pick up the saved server's id, and
/// notice when the attempt fails. Runs after every batch of events and answers.
pub(crate) fn update(app: &mut App) {
    if let Some(req) = app.welcome.save_req {
        if let Some(res) = app.take_result(req) {
            app.welcome.save_req = None;
            app.welcome.saving = false;
            match res {
                Ok(Reply::Server(saved)) => {
                    app.welcome.saved_id = saved.id.clone();
                    app.welcome.server.id = saved.id;
                }
                Ok(_) => {}
                Err(e) => app.welcome.failure = Some(Failure { raw: e, sasl: false }),
            }
        }
    }
    let w = &app.welcome;
    if w.step != Step::Connect || w.saving || w.failure.is_some() {
        return;
    }
    let Some(c) = app.conns.get(&w.saved_id) else { return };
    let (status, error) = (c.status, c.error.clone());
    let sasl_failed = sasl_error(c, w.baseline);
    let plain = w.server.sasl == SaslMechanism::Plain;
    let saved_id = w.saved_id.clone();
    match status {
        ConnStatus::Connecting | ConnStatus::Registering => app.welcome.live = true,
        ConnStatus::Disconnected if app.welcome.live && error.is_some() => {
            let raw = [sasl_failed, error].into_iter().flatten().collect::<Vec<_>>().join(" \u{2014} ");
            app.welcome.failure = Some(Failure { raw, sasl: plain });
            // Stop the backoff loop; the user decides whether to retry or edit.
            app.env.emit(Effect::Disconnect { server: saved_id, reason: None });
        }
        _ => {}
    }
}
