// The Rust core, reached over the C ABI in crates/hearth-ffi. Commands take
// the same names and arguments as the Tauri frontend's `invoke` calls, so
// this file plays the role of src/lib/api.ts.

import CHearth
import Foundation

struct CoreError: Error, LocalizedError, CustomStringConvertible {
  let message: String
  var errorDescription: String? { message }
  var description: String { message }
}

/// What the model needs from the core. Tests substitute a fake.
@MainActor
protocol Backend: AnyObject {
  /// Raw JSON result of a command.
  func call(_ command: String, _ args: [String: Any]) async throws -> Data
  /// Where event batches go. Set once, before anything connects.
  var onEvents: (([IrcEvent]) -> Void)? { get set }
  var closeToTray: Bool { get }
}

extension Backend {
  func call<T: Decodable>(_ command: String, _ args: [String: Any] = [:], as: T.Type = T.self) async throws -> T {
    let data = try await call(command, args)
    return try JSONDecoder().decode(T.self, from: data)
  }

  func run(_ command: String, _ args: [String: Any] = [:]) async throws {
    _ = try await call(command, args)
  }
}

/// Typed wrappers, one per core command.
extension Backend {
  func getConfig() async throws -> Config { try await call("get_config") }
  func saveServer(_ s: ServerConfig) async throws -> ServerConfig {
    try await call("save_server", ["server": try jsonObject(s)])
  }
  func deleteServer(_ id: String) async throws { try await run("delete_server", ["id": id]) }
  func saveSettings(_ s: Settings) async throws { try await run("save_settings", ["settings": try jsonObject(s)]) }
  func connect(_ server: String) async throws { try await run("connect", ["server": server]) }
  func disconnect(_ server: String, reason: String? = nil) async throws {
    try await run("disconnect", ["server": server, "reason": reason ?? NSNull()])
  }
  func reconnect(_ server: String) async throws { try await run("reconnect", ["server": server]) }
  func reset() async throws { try await run("reset") }
  func send(_ conn: String, _ lines: [String]) async throws { try await run("send", ["conn": conn, "lines": lines]) }
  func history(_ conn: String, kind: String, target: String, line: String) async throws -> [IrcMessage] {
    try await call("history", ["conn": conn, "kind": kind, "target": target, "line": line])
  }
  /// From the local log; `target` is the case-folded buffer name.
  func localHistory(_ conn: String, target: String, before: Int? = nil, after: Int? = nil, limit: Int) async throws
    -> [IrcMessage]
  {
    try await call(
      "local_history",
      ["conn": conn, "target": target, "before": before ?? NSNull(), "after": after ?? NSNull(), "limit": limit])
  }
  func search(_ query: String, network: String?, target: String?, limit: Int = 200) async throws -> [SearchHit] {
    try await call(
      "search", ["query": query, "network": network ?? NSNull(), "target": target ?? NSNull(), "limit": limit])
  }
  func loadBuffers(_ conn: String) async throws -> [SavedBuffer] { try await call("load_buffers", ["conn": conn]) }
  func saveBuffers(_ conn: String, _ buffers: [SavedBuffer]) async throws {
    try await run("save_buffers", ["conn": conn, "buffers": try jsonObject(buffers)])
  }
  func clearLogs() async throws { try await run("clear_logs") }
  func linkPreview(_ url: String) async throws -> Preview { try await call("link_preview", ["url": url]) }
  /// A metadata avatar URL, fetched by the core, as a data: URL.
  func avatar(_ url: String) async throws -> String { try await call("avatar", ["url": url]) }
  func generateCert(_ server: String) async throws -> CertInfo { try await call("generate_cert", ["server": server]) }
  func certInfo(_ path: String) async throws -> CertInfo { try await call("cert_info", ["path": path]) }
  func uploadFile(server: String, url: String, path: String) async throws -> String {
    try await call("upload_file", ["server": server, "url": url, "path": path])
  }
  func uploadBytes(server: String, url: String, name: String, contentType: String, data: Data) async throws -> String {
    try await call(
      "upload_bytes",
      ["server": server, "url": url, "name": name, "contentType": contentType, "data": data.base64EncodedString()])
  }
}

func jsonObject<T: Encodable>(_ v: T) throws -> Any {
  try JSONSerialization.jsonObject(with: JSONEncoder().encode(v), options: [.fragmentsAllowed])
}

/// The real core, linked from libhearth_ffi.a.
@MainActor
final class RustCore: Backend {
  private var handle: OpaquePointer?
  private let events = EventBox()

  var onEvents: (([IrcEvent]) -> Void)? {
    get { events.sink }
    set { events.sink = newValue }
  }

  /// Opens the config and log in the same directories the Tauri build used,
  /// so switching frontends keeps servers, passwords and scrollback.
  init() throws {
    let support = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
    // HEARTH_DATA_DIR keeps a development copy away from the real profile.
    let dir = ProcessInfo.processInfo.environment["HEARTH_DATA_DIR"].map { URL(fileURLWithPath: $0, isDirectory: true) }
      ?? support.appendingPathComponent("app.hearth.irc", isDirectory: true)
    try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
    var error: UnsafeMutablePointer<CChar>?
    let ctx = Unmanaged.passUnretained(events).toOpaque()
    handle = hearth_open(dir.path, dir.path, { ctx, json in
      guard let ctx, let json else { return }
      let box = Unmanaged<EventBox>.fromOpaque(ctx).takeUnretainedValue()
      box.deliver(Data(bytes: json, count: strlen(json)))
    }, ctx, &error)
    if handle == nil {
      let msg = error.map { String(cString: $0) } ?? "could not start"
      if let error { hearth_string_free(error) }
      throw CoreError(message: msg)
    }
  }

  var closeToTray: Bool { handle.map { hearth_close_to_tray($0) } ?? false }

  func call(_ command: String, _ args: [String: Any]) async throws -> Data {
    guard let handle else { throw CoreError(message: "Hearth's core is closed") }
    let json = try JSONSerialization.data(withJSONObject: args)
    let argString = String(decoding: json, as: UTF8.self)
    return try await withCheckedThrowingContinuation { (cont: CheckedContinuation<Data, Error>) in
      let reply = Unmanaged.passRetained(ReplyBox(cont)).toOpaque()
      hearth_call(handle, command, argString, { ctx, ok, json in
        guard let ctx else { return }
        let box = Unmanaged<ReplyBox>.fromOpaque(ctx).takeRetainedValue()
        let data = json.map { Data(bytes: $0, count: strlen($0)) } ?? Data()
        box.resume(ok: ok, data: data)
      }, reply)
    }
  }

  /// Quit every connection and flush the log.
  func close() {
    guard let h = handle else { return }
    handle = nil
    hearth_close(h)
  }
}

private final class ReplyBox: @unchecked Sendable {
  let cont: CheckedContinuation<Data, Error>
  init(_ cont: CheckedContinuation<Data, Error>) { self.cont = cont }

  func resume(ok: Bool, data: Data) {
    if ok {
      cont.resume(returning: data)
    } else {
      let msg = (try? JSONDecoder().decode(String.self, from: data)) ?? String(decoding: data, as: UTF8.self)
      cont.resume(throwing: CoreError(message: msg))
    }
  }
}

/// Receives event batches on a core thread and hands them to the main actor.
private final class EventBox: @unchecked Sendable {
  @MainActor var sink: (([IrcEvent]) -> Void)?

  func deliver(_ data: Data) {
    let events: [IrcEvent]
    do {
      events = try JSONDecoder().decode([IrcEvent].self, from: data)
    } catch {
      NSLog("hearth: undecodable events: \(error)")
      return
    }
    DispatchQueue.main.async { MainActor.assumeIsolated { self.sink?(events) } }
  }
}
