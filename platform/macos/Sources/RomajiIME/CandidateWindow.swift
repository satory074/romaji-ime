import Cocoa

/// A custom candidate list shown below the caret. Deliberately NOT IMKCandidates
/// (which locks down font/colors/window-level and has rendering bugs on recent
/// macOS). A single shared, non-activating panel is reused for every controller
/// so we never accumulate NSWindow instances.
final class CandidateWindow {
    static let shared = CandidateWindow()

    private let panel: NSPanel
    private let field: NSTextField
    private let padding: CGFloat = 8

    private init() {
        field = NSTextField(labelWithString: "")
        field.isBezeled = false
        field.drawsBackground = false
        field.isEditable = false
        field.isSelectable = false
        field.maximumNumberOfLines = 0

        let content = NSView()
        content.wantsLayer = true
        content.layer?.backgroundColor = NSColor.windowBackgroundColor.cgColor
        content.layer?.cornerRadius = 6
        content.addSubview(field)

        panel = NSPanel(
            contentRect: NSRect(x: 0, y: 0, width: 10, height: 10),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered, defer: true)
        panel.level = .popUpMenu                 // above normal windows / fullscreen apps
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.hasShadow = true
        panel.isFloatingPanel = true
        panel.hidesOnDeactivate = false
        panel.contentView = content
    }

    /// Show `candidates` with `highlighted` emphasized, positioned just below the
    /// caret rectangle (screen coords). Pass an empty list to hide.
    func show(candidates: [String], highlighted: Int, caret: NSRect) {
        guard !candidates.isEmpty else { hide(); return }

        let attr = NSMutableAttributedString()
        let font = NSFont.systemFont(ofSize: 16)
        for (i, candidate) in candidates.enumerated() {
            let prefix = i < 9 ? "\(i + 1). " : "    "
            let line = "\(prefix)\(candidate)\n"
            let piece = NSMutableAttributedString(
                string: line,
                attributes: [.font: font, .foregroundColor: NSColor.labelColor])
            if i == highlighted {
                piece.addAttribute(
                    .backgroundColor, value: NSColor.selectedContentBackgroundColor,
                    range: NSRange(location: 0, length: (line as NSString).length))
            }
            attr.append(piece)
        }
        if attr.length > 0 {
            attr.deleteCharacters(in: NSRange(location: attr.length - 1, length: 1))  // trailing \n
        }
        present(attr, caret: caret)
    }

    /// Show a single greyed status line (e.g. "変換中…") while the AI is working.
    func showStatus(_ text: String, caret: NSRect) {
        let attr = NSAttributedString(
            string: text,
            attributes: [
                .font: NSFont.systemFont(ofSize: 14),
                .foregroundColor: NSColor.secondaryLabelColor,
            ])
        present(attr, caret: caret)
    }

    /// Lay out the panel to fit `attr`, position it below the caret, and show it.
    private func present(_ attr: NSAttributedString, caret: NSRect) {
        field.attributedStringValue = attr
        let bounds = attr.boundingRect(
            with: NSSize(width: 600, height: 0),
            options: [.usesLineFragmentOrigin, .usesFontLeading])
        let textSize = NSSize(width: ceil(bounds.width), height: ceil(bounds.height))
        field.frame = NSRect(origin: NSPoint(x: padding, y: padding), size: textSize)
        let panelSize = NSSize(width: textSize.width + padding * 2, height: textSize.height + padding * 2)
        panel.contentView?.frame = NSRect(origin: .zero, size: panelSize)
        panel.setContentSize(panelSize)

        // Top-left just below the caret line; fall back to the mouse location when
        // the app doesn't report a caret rect.
        let topLeft: NSPoint
        if caret == .zero {
            let m = NSEvent.mouseLocation
            topLeft = NSPoint(x: m.x, y: m.y - 20)
        } else {
            topLeft = NSPoint(x: caret.minX, y: caret.minY)
        }
        panel.setFrameTopLeftPoint(topLeft)
        panel.orderFront(nil)
    }

    func hide() {
        panel.orderOut(nil)
    }
}
