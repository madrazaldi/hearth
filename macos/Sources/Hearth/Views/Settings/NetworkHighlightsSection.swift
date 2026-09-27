import SwiftUI

/// Per-server highlight rules (NetworkHighlights.svelte). Saved as you type,
/// as long as every rule is valid.
struct NetworkHighlightsSection: View {
  @Environment(AppModel.self) private var app
  @Environment(\.palette) private var p
  let serverId: String
  let nick: String
  @LocalState private var words = ""
  @LocalState private var never = ""
  @LocalState private var sample = ""
  @LocalState private var sampleFrom = ""
  @LocalState private var loaded = false

  private var rules: [String] { settingsList(words, separators: "\n") }
  private var errors: [String] { rules.compactMap { w in ruleError(w).map { "\(w): \($0)" } } }
  private var valid: [String] { rules.filter { ruleError($0) == nil } }
  private var neverList: [String] { settingsList(never, separators: ", \t\n") }

  var body: some View {
    Section {
      VStack(alignment: .leading, spacing: 6) {
        Text("Extra highlight words")
        TextEditor(text: $words)
          .font(.body.monospaced())
          .autocorrectionDisabled()
          .frame(height: 56)
          .scrollContentBackground(.hidden)
          .padding(4)
          .background(RoundedRectangle(cornerRadius: 5).fill(Color(nsColor: .textBackgroundColor)))
          .overlay(RoundedRectangle(cornerRadius: 5).strokeBorder(Color(nsColor: .separatorColor)))
          .accessibilityLabel("Extra highlight words")
        if errors.isEmpty {
          SettingsNote(
            "One per line. Words match whole-word, ignoring case. Wrap a regular expression in slashes: `/\\bci (failed|broke)\\b/i`."
          )
        } else {
          ForEach(errors, id: \.self) { SettingsNote("Invalid pattern: \($0)", .error) }
        }
      }
      VStack(alignment: .leading, spacing: 6) {
        TextField("Never highlight from", text: $never, prompt: Text("ChanServ, *bot*, *!*@services.*"))
          .font(.body.monospaced())
          .autocorrectionDisabled()
        SettingsNote("Nicks or `nick!user@host` masks, such as bots that repeat your name.")
      }
      VStack(alignment: .leading, spacing: 6) {
        LabeledContent("Test a message") {
          HStack {
            TextField("Sender", text: $sampleFrom, prompt: Text("from"))
              .labelsHidden()
              .font(.body.monospaced())
              .autocorrectionDisabled()
              .frame(width: 110)
            TextField("Sample message", text: $sample, prompt: Text("Type a sample line"))
              .labelsHidden()
          }
        }
        if let result {
          switch result {
          case let .hit(why): SettingsNote("Highlights · \(why)", .success)
          case let .miss(why): SettingsNote("Doesn't highlight · \(why)")
          }
        }
      }
    } header: {
      Text("Highlights on this server")
    } footer: {
      SettingsNote("Added to your global highlight words. For a bouncer, these apply to all of its networks.")
    }
    .onAppear {
      guard !loaded else { return }
      loaded = true
      let h = app.networkHighlights(serverId)
      words = h.words.joined(separator: "\n")
      never = h.never.joined(separator: ", ")
    }
    .onChange(of: words) { save() }
    .onChange(of: never) { save() }
  }

  private var result: HighlightTest? {
    app.testHighlight(serverId: serverId, nick: nick, words: valid, never: neverList, text: sample, from: sampleFrom)
  }

  private func save() {
    // Keep what's saved until the edit is valid, rather than dropping the bad rule.
    guard loaded, errors.isEmpty else { return }
    let current = app.networkHighlights(serverId)
    if current.words == valid && current.never == neverList { return }
    app.setNetworkHighlights(serverId, words: valid, never: neverList)
  }
}
