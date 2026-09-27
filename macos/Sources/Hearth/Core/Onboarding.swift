// First-run helpers: network presets, input validation, and plain-language
// explanations for the ways a first connection can fail (onboarding.ts).

import Foundation

struct NetworkPreset: Identifiable, Hashable {
  enum Kind { case network, bouncer, custom }
  var id: String
  var label: String
  var blurb: String
  var kind: Kind
  var host: String
  var port: Int
  var tls: Bool
  /// NICKLEN, when the network is known to be stricter than the usual 30.
  var nickLen: Int?
  /// Whether services accept SASL; `false` skips the account step.
  var sasl: Bool?
  var accountNote: String?
  var channels: [String] = []
}

let presets: [NetworkPreset] = [
  NetworkPreset(
    id: "libera", label: "Libera.Chat", blurb: "Free and open-source communities", kind: .network,
    host: "irc.libera.chat", port: 6697, tls: true, nickLen: 16, sasl: true,
    accountNote:
      "Libera.Chat requires SASL when you connect from some address ranges, such as many VPNs, cloud hosts and Tor. It also protects a registered nickname and lets you into channels that only allow registered users.",
    channels: ["#libera"]),
  NetworkPreset(
    id: "oftc", label: "OFTC", blurb: "Debian, Tor and other free software projects", kind: .network,
    host: "irc.oftc.net", port: 6697, tls: true, sasl: false,
    accountNote:
      "OFTC doesn't use SASL. To use a registered nickname, identify with NickServ after connecting, or add a client certificate in the server settings.",
    channels: ["#oftc"]),
  NetworkPreset(
    id: "efnet", label: "EFnet", blurb: "One of the oldest networks, no services", kind: .network,
    host: "irc.efnet.org", port: 6697, tls: true, nickLen: 9, sasl: false,
    accountNote: "EFnet has no nickname services, so there's no account to log in to.", channels: ["#efnet"]),
  NetworkPreset(
    id: "rizon", label: "Rizon", blurb: "Large general-purpose network", kind: .network,
    host: "irc.rizon.net", port: 6697, tls: true, sasl: true,
    accountNote: "Log in if you have registered your nickname with NickServ.", channels: ["#rizon"]),
  NetworkPreset(
    id: "hackint", label: "hackint", blurb: "Hacker and maker communities", kind: .network,
    host: "irc.hackint.org", port: 6697, tls: true, sasl: true,
    accountNote: "Log in if you have registered your nickname with NickServ.", channels: ["#hackint"]),
  NetworkPreset(
    id: "soju", label: "soju bouncer", blurb: "Stays connected for you and keeps history", kind: .bouncer,
    host: "", port: 6697, tls: true, sasl: true,
    accountNote: "soju needs the username and password of your bouncer account."),
  NetworkPreset(
    id: "custom", label: "Custom server", blurb: "Any other server, with every option", kind: .custom,
    host: "", port: 6697, tls: true),
]

private let special = "[]\\`_^{|}"

/// Checks a nickname against the RFC 2812 grammar most servers still enforce.
func nickError(_ nick: String, maxLen: Int = 30, network: String = "this network") -> String? {
  if nick.isEmpty { return "Choose a nickname." }
  if nick.contains(where: \.isWhitespace) { return "Nicknames can't contain spaces." }
  if let f = nick.first, f.isASCII && (f.isNumber || f == "-") { return "Nicknames can't start with a digit or a dash." }
  if let bad = nick.first(where: { !($0.isASCII && ($0.isLetter || $0.isNumber || $0 == "-")) && !special.contains($0) }) {
    return "“\(bad)” isn't allowed. Use letters, digits and - _ [ ] { } \\ | ^ `"
  }
  if nick.count > maxLen { return "Keep it to \(maxLen) characters or fewer on \(network)." }
  return nil
}

/// Splits a channel list typed by hand; bare names get a `#`.
func parseChannels(_ text: String) -> [String] {
  var out: [String] = []
  for raw in text.split(whereSeparator: { $0.isWhitespace || $0 == "," }) {
    let c = String(raw)
    let name = c.first.map { "#&+!".contains($0) } == true ? c : "#" + c
    if !out.contains(where: { $0.lowercased() == name.lowercased() }) { out.append(name) }
  }
  return out
}

struct Friendly {
  var title: String
  var hint: String
}

/// Turns a core connection error (an anyhow chain, or the server's ERROR
/// reason) into something a newcomer can act on.
func explainError(_ raw: String, host: String, port: Int, tls: Bool, sasl: Bool) -> Friendly {
  let e = raw.lowercased()
  func has(_ re: Regex<Substring>) -> Bool { e.firstMatch(of: re) != nil }
  if has(/sasl|authenticat/) {
    return sasl
      ? Friendly(title: "Your account name or password was not accepted", hint: "Check them and try again.")
      : Friendly(
        title: "This network wants you to log in first",
        hint:
          "Connections from your address need an account (SASL). Add your account, or connect from another network.")
  }
  if has(/lookup|resolve|nodename|name or service|no such host|dns/) {
    return Friendly(title: "Couldn't find \(host)", hint: "Check the server address for typos.")
  }
  if has(/refused/) {
    return Friendly(
      title: "The server refused the connection",
      hint: "Nothing is accepting connections on port \(port). Check the port, and whether the server expects TLS.")
  }
  if has(/certificate|tls|handshake|unknownissuer|notvalidforname|expired/) {
    return Friendly(
      title: "Couldn't set up a secure connection",
      hint: tls
        ? "The server's certificate wasn't trusted, or port \(port) doesn't speak TLS. TLS ports are usually 6697."
        : "The server may require TLS. Turn TLS on and use port 6697.")
  }
  if has(/timed out connecting|unreachable|network is down|no route/) {
    return Friendly(title: "The server didn't answer", hint: "Check your internet connection and the server address.")
  }
  if has(/registration timed out/) {
    return Friendly(title: "The server never finished logging you in", hint: "It may be busy. Try again in a moment.")
  }
  if has(/[kgzd]-?line|banned|ban\b/) {
    return Friendly(
      title: "This network is refusing connections from your address", hint: "This is set by the network, not Hearth.")
  }
  if has(/too many|throttl|reconnecting too fast/) {
    return Friendly(title: "Too many connections from your address", hint: "Wait a minute, then try again.")
  }
  return Friendly(title: "Couldn't connect", hint: "The details below may help.")
}
