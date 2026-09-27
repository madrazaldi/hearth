import SwiftUI

/// SwiftUI's `State`. Newer SDKs make `@State` a macro whose plugin ships only
/// with Xcode, so spelling it through a typealias lets the Command Line Tools
/// build the app too. Use `@LocalState` in place of `@State`, and avoid other
/// SwiftUI macros (`#Preview`, `@Entry`, `@Previewable`).
typealias LocalState = SwiftUI.State
