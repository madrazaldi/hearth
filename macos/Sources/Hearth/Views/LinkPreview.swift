// A card under a message for its first link (LinkPreview.svelte). The Rust side
// fetches it, so images arrive as data URLs.

import SwiftUI

struct LinkPreviewView: View {
  let url: String
  let nsfw: Bool
  @Environment(AppModel.self) private var app
  @Environment(\.palette) private var p
  @Environment(\.openURL) private var openURL
  @LocalState private var preview: Preview?
  @LocalState private var image: NSImage?
  @LocalState private var revealed = false

  var body: some View {
    Group {
      if let preview {
        switch preview.kind {
        case "image" where image != nil: imageCard(preview)
        case "media": mediaCard(preview)
        case "page" where preview.title != nil || preview.description != nil: pageCard(preview)
        default: EmptyView()
        }
      }
    }
    .task(id: url) {
      let p = await app.preview(url)
      image = p?.image.flatMap(imageFromDataURL)
      preview = p
    }
  }

  private func openLink() {
    if let u = URL(string: url) { openURL(u) }
  }

  private func imageCard(_ pv: Preview) -> some View {
    let img = image!
    let maxW: CGFloat = 360
    let maxH: CGFloat = 280
    let s = img.size
    let scale = min(1, maxW / max(s.width, 1), maxH / max(s.height, 1))
    let blurred = nsfw && !revealed
    return Button {
      if blurred { revealed = true } else { openLink() }
    } label: {
      Image(nsImage: img)
        .resizable()
        .interpolation(.high)
        .frame(width: s.width * scale, height: s.height * scale)
        .blur(radius: blurred ? 24 : 0)
        .clipShape(RoundedRectangle(cornerRadius: 8))
        .overlay {
          if blurred {
            Text("NSFW · click to show").font(.system(size: 12, weight: .semibold)).foregroundStyle(.white)
              .padding(.horizontal, 10).padding(.vertical, 5).background(Capsule().fill(.black.opacity(0.55)))
          }
        }
        .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(p.line))
    }
    .buttonStyle(.plain)
    .help(blurred ? "Show image" : url)
  }

  private func mediaCard(_ pv: Preview) -> some View {
    Button(action: openLink) {
      HStack(spacing: 10) {
        Image(systemName: pv.mime?.hasPrefix("audio/") == true ? "music.note" : "play.fill")
          .font(.system(size: 14))
          .foregroundStyle(p.text2)
          .frame(width: 32, height: 32)
          .background(RoundedRectangle(cornerRadius: 7).fill(p.active))
        VStack(alignment: .leading, spacing: 2) {
          Text(fileName(url)).font(.system(size: 12.5, weight: .medium)).foregroundStyle(p.text).lineLimit(1)
          Text(mediaLabel(pv)).font(.system(size: 11)).foregroundStyle(p.text3)
        }
      }
      .padding(8)
      .frame(maxWidth: 360, alignment: .leading)
      .background(RoundedRectangle(cornerRadius: 9).fill(p.raised))
      .overlay(RoundedRectangle(cornerRadius: 9).strokeBorder(p.line))
    }
    .buttonStyle(.plain)
    .help(url)
  }

  private func pageCard(_ pv: Preview) -> some View {
    Button(action: openLink) {
      HStack(alignment: .top, spacing: 10) {
        VStack(alignment: .leading, spacing: 3) {
          if let site = pv.site {
            Text(site).font(.system(size: 11, weight: .medium)).foregroundStyle(p.text3).lineLimit(1)
          }
          if let title = pv.title {
            Text(title).font(.system(size: 13, weight: .semibold)).foregroundStyle(p.accent).lineLimit(2)
          }
          if let d = pv.description {
            Text(d).font(.system(size: 12)).foregroundStyle(p.text2).lineLimit(3)
          }
        }
        Spacer(minLength: 0)
        if let image, !(nsfw && !revealed) {
          Image(nsImage: image).resizable().aspectRatio(contentMode: .fill)
            .frame(width: 72, height: 72).clipShape(RoundedRectangle(cornerRadius: 6))
        }
      }
      .padding(10)
      .frame(maxWidth: 440, alignment: .leading)
      .background(RoundedRectangle(cornerRadius: 9).fill(p.raised))
      .overlay(alignment: .leading) { Rectangle().fill(p.accent.opacity(0.7)).frame(width: 3).padding(.vertical, 1) }
      .overlay(RoundedRectangle(cornerRadius: 9).strokeBorder(p.line))
      .clipShape(RoundedRectangle(cornerRadius: 9))
    }
    .buttonStyle(.plain)
    .help(url)
  }
}
