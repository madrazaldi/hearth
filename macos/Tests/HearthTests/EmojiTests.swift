// Emoji shortcodes (emoji.test.ts).

import Foundation
import Testing

@testable import Hearth

@MainActor @Suite("emoji")
struct EmojiTests {
  @Test("finds the query being typed")
  func query() {
    #expect(Emojis.query(":j")! == (0, "j"))
    #expect(Emojis.query("lol :Jo")! == (4, "jo"))
    #expect(Emojis.query("nice :+1")! == (5, "+1"))
  }

  @Test("ignores colons that do not start a word")
  func notQuery() {
    for s in ["at 12:30", "https://x", "nick:", ":joy: ", ":"] {
      #expect(Emojis.query(s) == nil, "\(s)")
    }
  }

  @Test("ranks prefix matches first, then names containing the query")
  func search() {
    #expect(Emojis.search("jo").first?.name == "joy")
    #expect(Emojis.search("j").count == 7)
    let eyes = Emojis.search("eyes").map(\.name)
    #expect(eyes.first == "eyes")
    #expect(eyes.contains("heart_eyes"))
    #expect(Emojis.search("zzzqqq").isEmpty)
  }

  @Test("expands a complete shortcode")
  func typed() {
    #expect(Emojis.named("joy") == "😂")
    #expect(Emojis.typed("haha :joy:")! == (5, "😂"))
    #expect(Emojis.typed(":THUMBSUP:")! == (0, "👍"))
    #expect(Emojis.typed("haha :notanemoji:") == nil)
    #expect(Emojis.typed("a:joy:") == nil)
    // UTF-16 offsets, as NSTextView counts them.
    #expect(Emojis.typed("🙂 :joy:")!.start == 3)
  }
}
