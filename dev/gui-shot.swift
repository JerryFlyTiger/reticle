// Print "<windowID> <x> <y> <width> <height>" for the first on-screen window
// belonging to a process whose name contains the argument, case-insensitively.
//
// Used by dev/gui-shot.sh. This exists because the obvious route -- asking
// System Events for `window 1 of process "reticle"` -- needs Accessibility
// permission, and returns a bare index error when it is missing, which reads
// like "the app has no window" rather than "you lack permission".
// CGWindowListCopyWindowInfo returns owner name and bounds without any
// permission at all; only the pixels are gated.

import CoreGraphics
import Foundation

let target = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : ""
if target.isEmpty {
    FileHandle.standardError.write(Data("usage: gui-shot <process-name-substring>\n".utf8))
    exit(2)
}

guard
    let list = CGWindowListCopyWindowInfo(
        [.optionOnScreenOnly, .excludeDesktopElements], kCGNullWindowID) as? [[String: Any]]
else {
    FileHandle.standardError.write(Data("could not read the window list\n".utf8))
    exit(2)
}

for window in list {
    guard let owner = window[kCGWindowOwnerName as String] as? String,
        owner.lowercased().contains(target.lowercased()),
        let id = window[kCGWindowNumber as String] as? Int,
        let bounds = window[kCGWindowBounds as String] as? [String: Any],
        let x = bounds["X"] as? Double,
        let y = bounds["Y"] as? Double,
        let width = bounds["Width"] as? Double,
        let height = bounds["Height"] as? Double,
        // Skip menu-bar items and other small chrome the process also owns.
        width > 100, height > 100
    else { continue }
    print("\(id) \(Int(x)) \(Int(y)) \(Int(width)) \(Int(height))")
    exit(0)
}

exit(1)
