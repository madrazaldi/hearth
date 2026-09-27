//! Registration: CAP negotiation, SASL, nick fallback and what follows the
//! welcome.

mod common;

use common::*;
use hearth_core::config::{SaslMechanism, ServerConfig};
use hearth_core::irc::events::Event;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

async fn start(configure: impl FnOnce(&mut ServerConfig)) -> (Harness, Peer) {
    let h = Harness::new().await;
    let mut server = h.server();
    configure(&mut server);
    h.mgr.connect(&server);
    let p = Peer::accept(&h.listener).await;
    (h, p)
}

fn plain(user: &str, password: &str) -> impl FnOnce(&mut ServerConfig) {
    let (user, password) = (user.to_string(), password.to_string());
    move |s| {
        s.sasl = SaslMechanism::Plain;
        s.sasl_username = user;
        s.sasl_password = password;
    }
}

fn caps_of(line: &str) -> BTreeSet<String> {
    line.split_once(" :").unwrap().1.split_whitespace().map(str::to_string).collect()
}

async fn registered_nick(h: &Harness) -> String {
    match wait_event(&h.events, "registration", |e| matches!(e, Event::Registered { .. })).await {
        Event::Registered { nick, .. } => nick,
        _ => unreachable!(),
    }
}

/// The capability set carried by the latest Caps event, once it satisfies `pred`.
async fn caps_event(h: &Harness, pred: impl Fn(&[String]) -> bool) -> Vec<String> {
    let ev = wait_event(&h.events, "a caps event", |e| matches!(e, Event::Caps { caps, .. } if pred(caps))).await;
    match ev {
        Event::Caps { caps, .. } => caps,
        _ => unreachable!(),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn opening_lines_carry_pass_and_user() {
    let (h, mut p) = start(|s| {
        s.password = "serverpw".into();
        s.username = "ident".into();
        s.realname = "Real Name".into();
    })
    .await;
    assert_eq!(p.line().await, "CAP LS 302");
    assert_eq!(p.line().await, "PASS serverpw");
    assert_eq!(p.line().await, "NICK me");
    assert_eq!(p.line().await, "USER ident 0 * :Real Name");
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn multiline_cap_ls_requests_only_wanted_caps() {
    let (h, mut p) = start(|_| {}).await;
    p.expect("USER").await;
    p.send(":srv CAP * LS * :account-notify away-notify batch cap-notify chghost draft/chathistory").await;
    p.send(":srv CAP * LS * :echo-message extended-join invite-notify labeled-response message-tags").await;
    // Nothing is requested until the last line of the listing.
    p.assert_quiet().await;
    p.send(":srv CAP * LS :multi-prefix server-time setname sasl=PLAIN znc.in/self-message draft/unknown=1").await;
    let req = p.expect("CAP REQ").await;
    assert!(req.len() < 512);
    let want = "account-notify away-notify batch cap-notify chghost draft/chathistory echo-message extended-join \
                invite-notify labeled-response message-tags multi-prefix server-time setname";
    let want: BTreeSet<String> = want.split_whitespace().map(str::to_string).collect();
    // sasl is left out: no credentials are configured.
    assert_eq!(caps_of(&req), want);

    // CAP END waits for the ACK.
    p.assert_quiet().await;
    p.send(&format!(":srv CAP me ACK :{}", req.split_once(" :").unwrap().1)).await;
    assert_eq!(p.line().await, "CAP END");
    p.welcome().await;
    let caps = match wait_event(&h.events, "registration", |e| matches!(e, Event::Registered { .. })).await {
        Event::Registered { caps, .. } => caps,
        _ => unreachable!(),
    };
    assert_eq!(caps.into_iter().collect::<BTreeSet<_>>(), want);
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn every_wanted_cap_is_requested_in_lines_under_the_limit() {
    let (h, mut p) = start(plain("acct", "pw")).await;
    p.expect("CAP LS").await;
    let all = "account-notify account-tag away-notify batch cap-notify chghost draft/account-registration \
               draft/channel-rename draft/chathistory draft/event-playback draft/extended-monitor \
               draft/message-redaction draft/metadata-2 draft/multiline draft/read-marker echo-message \
               extended-join extended-monitor invite-notify labeled-response message-tags multi-prefix sasl \
               server-time setname soju.im/bouncer-networks soju.im/bouncer-networks-notify soju.im/search \
               standard-replies userhost-in-names";
    p.send(&format!(":srv CAP * LS :{all}")).await;
    let want: BTreeSet<String> = all.split_whitespace().map(str::to_string).collect();
    let mut got = BTreeSet::new();
    let mut reqs = 0;
    while got != want {
        let req = p.expect("CAP REQ").await;
        assert!(req.len() < 512, "{} bytes", req.len());
        got.extend(caps_of(&req));
        reqs += 1;
    }
    assert_eq!(reqs, 2, "the list is split, not sent one cap at a time");
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn nak_still_ends_negotiation() {
    let (h, mut p) = start(|_| {}).await;
    p.expect("CAP LS").await;
    p.send(":srv CAP * LS :server-time batch").await;
    let req = p.expect("CAP REQ").await;
    p.send(&format!(":srv CAP * NAK :{}", req.split_once(" :").unwrap().1)).await;
    assert_eq!(p.line().await, "CAP END");
    p.welcome().await;
    registered_nick(&h).await;
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn no_caps_at_all_ends_immediately() {
    let (h, mut p) = start(plain("acct", "pw")).await;
    p.expect("USER").await;
    p.send(":srv CAP * LS :").await;
    assert_eq!(p.line().await, "CAP END");
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn cap_notify_adds_and_removes_caps() {
    let (h, mut p) = start(plain("acct", "pw")).await;
    p.register("cap-notify server-time").await;
    registered_nick(&h).await;

    // New caps are requested, except sasl once registered.
    p.send(":srv CAP me NEW :away-notify sasl=PLAIN made-up").await;
    assert_eq!(p.line().await, "CAP REQ :away-notify");
    p.send(":srv CAP me ACK :away-notify").await;
    let caps = caps_event(&h, |c| c.iter().any(|c| c == "away-notify")).await;
    assert_eq!(caps, ["away-notify", "cap-notify", "server-time"]);
    p.assert_quiet().await;

    // Offering an enabled cap again doesn't request it twice.
    p.send(":srv CAP me NEW :server-time").await;
    p.assert_quiet().await;

    p.send(":srv CAP me DEL :away-notify").await;
    let caps = caps_event(&h, |c| c.len() == 2).await;
    assert_eq!(caps, ["cap-notify", "server-time"]);

    // A removal can also arrive as an ACK with a '-' prefix.
    p.send(":srv CAP me ACK :-server-time").await;
    let caps = caps_event(&h, |c| c.len() == 1).await;
    assert_eq!(caps, ["cap-notify"]);
    // Registration is over: none of this sends CAP END again.
    p.assert_quiet().await;
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn isupport_tokens_are_parsed_and_unescaped() {
    let (h, mut p) = start(|_| {}).await;
    p.register("").await;
    p.send(r":srv 005 me NETWORK=Foo\x20Net STATUSMSG=@+ EXCEPTS :are supported").await;
    p.send(":srv 005 me -EXCEPTS NICKLEN=9 :are supported").await;
    let isupport = wait_event(&h.events, "isupport", |e| {
        matches!(e, Event::Isupport { isupport, .. } if isupport.contains_key("NICKLEN"))
    })
    .await;
    let Event::Isupport { isupport, .. } = isupport else { unreachable!() };
    assert_eq!(isupport["NETWORK"], "Foo Net");
    assert_eq!(isupport["STATUSMSG"], "@+");
    assert_eq!(isupport["CASEMAPPING"], "ascii", "earlier lines are kept");
    assert!(!isupport.contains_key("EXCEPTS"), "negated tokens are removed");
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn nick_in_use_falls_back_during_registration_only() {
    let (h, mut p) = start(|s| {
        s.autojoin = vec!["#a".into()];
        s.perform = vec!["PRIVMSG NickServ :hi from $nick".into()];
    })
    .await;
    p.expect("CAP LS").await;
    p.send(":srv CAP * LS :").await;
    p.expect("CAP END").await;
    p.send(":srv 433 * me :Nickname is already in use").await;
    assert_eq!(p.line().await, "NICK me_");
    p.send(":srv 432 * me_ :Erroneous nickname").await;
    assert_eq!(p.line().await, "NICK me__");
    p.send(":srv 001 me__ :Welcome").await;
    p.send(":srv 422 me__ :No MOTD").await;
    assert_eq!(registered_nick(&h).await, "me__");
    assert_eq!(p.line().await, "JOIN #a");
    assert_eq!(p.line().await, "PRIVMSG NickServ :hi from me__");

    // Once registered, a failed /nick is the user's business.
    p.send(":srv 433 me__ taken :Nickname is already in use").await;
    p.assert_quiet().await;
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn nick_fallback_at_the_length_limit_still_changes_the_nick() {
    let long = "abcdefghijklmnopqrstuvwxyz0123";
    let (h, mut p) = start(|s| s.nick = long.into()).await;
    p.expect("CAP LS").await;
    p.send(":srv CAP * LS :").await;
    p.expect("CAP END").await;
    let mut seen = vec![long.to_string()];
    for _ in 0..3 {
        p.send(&format!(":srv 433 * {} :Nickname is already in use", seen.last().unwrap())).await;
        let nick = p.expect("NICK ").await.trim_start_matches("NICK ").to_string();
        assert!(nick.chars().count() <= 30, "{nick}");
        assert!(!seen.contains(&nick), "retried {nick} again");
        seen.push(nick);
    }
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn autojoin_is_sent_in_groups_of_ten() {
    let chans: Vec<String> = (0..12).map(|i| format!("#c{i}")).collect();
    let list = chans.clone();
    let (h, mut p) = start(move |s| s.autojoin = list).await;
    p.register("").await;
    assert_eq!(p.line().await, format!("JOIN {}", chans[..10].join(",")));
    assert_eq!(p.line().await, format!("JOIN {}", chans[10..].join(",")));
    // A later MOTD end (e.g. /motd) doesn't join again.
    p.send(":srv 376 me :End of MOTD").await;
    p.assert_quiet().await;
    h.mgr.disconnect_all();
}

async fn sasl_start(p: &mut Peer, ls: &str) {
    p.expect("CAP LS").await;
    p.send(&format!(":srv CAP * LS :{ls}")).await;
    let req = p.expect("CAP REQ").await;
    assert!(caps_of(&req).contains("sasl"), "{req}");
    p.send(&format!(":srv CAP * ACK :{}", req.split_once(" :").unwrap().1)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn plain_payload_is_split_into_400_byte_chunks() {
    // 10 bytes of "acct\0acct\0" plus 290 of password: exactly 400 base64 bytes.
    let password = "p".repeat(290);
    let (h, mut p) = start(plain("acct", &password)).await;
    sasl_start(&mut p, "sasl").await;
    assert_eq!(p.line().await, "AUTHENTICATE PLAIN");
    p.send("AUTHENTICATE +").await;
    let chunk = p.line().await;
    assert_eq!(chunk.len(), "AUTHENTICATE ".len() + 400);
    // A final chunk of exactly 400 bytes must be followed by an empty one.
    assert_eq!(p.line().await, "AUTHENTICATE +");
    let payload = base64_decode(chunk.trim_start_matches("AUTHENTICATE "));
    assert_eq!(payload, format!("acct\0acct\0{password}").into_bytes());
    p.send(":srv 900 me me!u@h acct :You are now logged in as acct").await;
    p.send(":srv 903 me :SASL authentication successful").await;
    assert_eq!(p.line().await, "CAP END");
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn sasl_failures_end_negotiation_and_register_anyway() {
    for numeric in ["902", "904", "905", "906"] {
        let (h, mut p) = start(plain("acct", "pw")).await;
        sasl_start(&mut p, "sasl=PLAIN").await;
        assert_eq!(p.line().await, "AUTHENTICATE PLAIN");
        p.send("AUTHENTICATE +").await;
        p.expect("AUTHENTICATE ").await;
        p.send(&format!(":srv {numeric} me :SASL failed")).await;
        assert_eq!(p.line().await, "CAP END", "after {numeric}");
        p.welcome().await;
        registered_nick(&h).await;
        // The failure reaches the UI.
        h.event(|m| m.command == numeric).await;
        h.mgr.disconnect_all();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn scram_failure_without_plain_offered_gives_up() {
    let (h, mut p) = start(plain("acct", "pw")).await;
    sasl_start(&mut p, "sasl=SCRAM-SHA-256").await;
    assert_eq!(p.line().await, "AUTHENTICATE SCRAM-SHA-256");
    p.send("AUTHENTICATE +").await;
    p.expect("AUTHENTICATE ").await;
    p.send(":srv 908 me SCRAM-SHA-256 :are available SASL mechanisms").await;
    p.send(":srv 904 me :SASL authentication failed").await;
    assert_eq!(p.line().await, "CAP END");
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn plain_fallback_is_tried_only_once() {
    let (h, mut p) = start(plain("acct", "pw")).await;
    sasl_start(&mut p, "sasl=SCRAM-SHA-256,PLAIN").await;
    assert_eq!(p.line().await, "AUTHENTICATE SCRAM-SHA-256");
    p.send(":srv 904 me :SASL authentication failed").await;
    assert_eq!(p.line().await, "AUTHENTICATE PLAIN");
    p.send("AUTHENTICATE +").await;
    p.expect("AUTHENTICATE ").await;
    p.send(":srv 904 me :SASL authentication failed").await;
    assert_eq!(p.line().await, "CAP END");
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn sasl_without_a_mechanism_list_uses_plain() {
    let (h, mut p) = start(plain("", "pw")).await;
    sasl_start(&mut p, "sasl").await;
    assert_eq!(p.line().await, "AUTHENTICATE PLAIN");
    p.send("AUTHENTICATE +").await;
    let line = p.line().await;
    // With no account name the nick is used.
    assert_eq!(base64_decode(line.trim_start_matches("AUTHENTICATE ")), b"me\0me\0pw");
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn sasl_is_skipped_without_credentials() {
    let (h, mut p) = start(plain("acct", "")).await;
    p.expect("USER").await;
    p.send(":srv CAP * LS :sasl=PLAIN server-time").await;
    assert_eq!(p.line().await, "CAP REQ :server-time");
    p.send(":srv CAP * ACK :server-time").await;
    assert_eq!(p.line().await, "CAP END");
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn external_sends_an_empty_response() {
    let (h, mut p) = start(|s| {
        s.sasl = SaslMechanism::External;
        // A password is ignored: the certificate is the credential.
        s.sasl_password = "pw".into();
        s.client_cert = "/nonexistent/cert.pem".into();
    })
    .await;
    sasl_start(&mut p, "sasl=EXTERNAL,SCRAM-SHA-256,PLAIN").await;
    assert_eq!(p.line().await, "AUTHENTICATE EXTERNAL");
    p.send("AUTHENTICATE +").await;
    assert_eq!(p.line().await, "AUTHENTICATE +");
    p.send(":srv 903 me :SASL authentication successful").await;
    assert_eq!(p.line().await, "CAP END");
    h.mgr.disconnect_all();
}

type HmacSha256 = Hmac<Sha256>;

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = HmacSha256::new_from_slice(key).unwrap();
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// Server side of a SCRAM-SHA-256 exchange. Returns the client-final
/// message's proof check and the server-final message.
fn scram_server(password: &str, client_first: &str, server_first: &str, client_final: &str) -> (bool, String) {
    let bare = client_first.strip_prefix("n,,").unwrap();
    let attr = |msg: &str, k: &str| msg.split(',').find_map(|kv| kv.strip_prefix(&format!("{k}="))).unwrap().to_string();
    let salt = base64_decode(&attr(server_first, "s"));
    let iterations: u32 = attr(server_first, "i").parse().unwrap();
    let mut salted = [0u8; 32];
    pbkdf2::pbkdf2::<HmacSha256>(password.as_bytes(), &salt, iterations, &mut salted).unwrap();
    let client_key = hmac(&salted, b"Client Key");
    let stored_key = Sha256::digest(&client_key);
    let (without_proof, proof) = client_final.rsplit_once(",p=").unwrap();
    let auth = format!("{bare},{server_first},{without_proof}");
    let signature = hmac(&stored_key, auth.as_bytes());
    let expected: Vec<u8> = client_key.iter().zip(&signature).map(|(a, b)| a ^ b).collect();
    let server_sig = hmac(&hmac(&salted, b"Server Key"), auth.as_bytes());
    (base64_decode(proof) == expected, format!("v={}", base64_encode(&server_sig)))
}

/// Run SCRAM up to the server-final message, which `tamper` may alter.
async fn scram_exchange(p: &mut Peer, tamper: impl Fn(String) -> String) {
    sasl_start(p, "sasl=SCRAM-SHA-256,PLAIN").await;
    assert_eq!(p.line().await, "AUTHENTICATE SCRAM-SHA-256");
    p.send("AUTHENTICATE +").await;
    let first = String::from_utf8(base64_decode(p.line().await.trim_start_matches("AUTHENTICATE "))).unwrap();
    let cnonce = first.split(",r=").nth(1).unwrap();
    let server_first = format!("r={cnonce}srvnonce,s={},i=4096", base64_encode(b"salty"));
    p.send(&format!("AUTHENTICATE {}", base64_encode(server_first.as_bytes()))).await;
    let fin = String::from_utf8(base64_decode(p.line().await.trim_start_matches("AUTHENTICATE "))).unwrap();
    let (ok, server_final) = scram_server("correct horse", &first, &server_first, &fin);
    assert!(ok, "client proof must verify");
    p.send(&format!("AUTHENTICATE {}", base64_encode(tamper(server_final).as_bytes()))).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn scram_completes_with_mutual_authentication() {
    let (h, mut p) = start(plain("acct", "correct horse")).await;
    scram_exchange(&mut p, |f| f).await;
    // The server proved itself; the client's last word is empty.
    assert_eq!(p.line().await, "AUTHENTICATE +");
    p.send(":srv 903 me :SASL authentication successful").await;
    assert_eq!(p.line().await, "CAP END");
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn scram_aborts_on_a_forged_server_signature() {
    let (h, mut p) = start(plain("acct", "correct horse")).await;
    scram_exchange(&mut p, |_| format!("v={}", base64_encode(&[0u8; 32]))).await;
    assert_eq!(p.line().await, "AUTHENTICATE *");
    let fail = h.event(|m| m.command == "FAIL").await;
    assert_eq!(fail.param(0), "AUTHENTICATE");
    assert!(fail.param(2).contains("signature"), "{fail:?}");
    p.send(":srv 906 me :SASL authentication aborted").await;
    assert_eq!(p.line().await, "CAP END");
    h.mgr.disconnect_all();
}

#[tokio::test(flavor = "multi_thread")]
async fn scram_aborts_on_a_foreign_nonce() {
    let (h, mut p) = start(plain("acct", "pw")).await;
    sasl_start(&mut p, "sasl=SCRAM-SHA-256").await;
    p.expect("AUTHENTICATE SCRAM").await;
    p.send("AUTHENTICATE +").await;
    p.expect("AUTHENTICATE ").await;
    let server_first = format!("r=notyours,s={},i=4096", base64_encode(b"salt"));
    p.send(&format!("AUTHENTICATE {}", base64_encode(server_first.as_bytes()))).await;
    assert_eq!(p.line().await, "AUTHENTICATE *");
    p.send(":srv 906 me :SASL authentication aborted").await;
    assert_eq!(p.line().await, "CAP END");
    h.mgr.disconnect_all();
}
