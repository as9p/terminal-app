import AppKit
import CoreMedia
import CoreVideo
import Foundation
import ScreenCaptureKit

// Measures the time from a key press to the first window frame that changed.
// Usage: keylat <pid> <rounds> <raw csv path>
// Types one "." then one backspace per round. Stops before any key if another window is on top.

let periodKey: CGKeyCode = 47
let deleteKey: CGKeyCode = 51

func machMilliseconds(_ ticks: UInt64) -> Double {
    var info = mach_timebase_info_data_t()
    mach_timebase_info(&info)
    return Double(ticks) * Double(info.numer) / Double(info.denom) / 1_000_000
}

func topWindowID() -> CGWindowID? {
    let options: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
    guard let list = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] else { return nil }
    for window in list where (window[kCGWindowLayer as String] as? Int) == 0 {
        if let alpha = window[kCGWindowAlpha as String] as? Double, alpha == 0 { continue }
        return window[kCGWindowNumber as String] as? CGWindowID
    }
    return nil
}

/// True when the target is the top window, looking past small windows of the same app (dialogs, size overlays).
/// Any window from another app, or a large window of the same app, above the target means false.
func targetIsOnTop(_ target: CGWindowID, pid: pid_t) -> Bool {
    let options: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
    guard let list = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] else { return false }
    for window in list where (window[kCGWindowLayer as String] as? Int) == 0 {
        if let alpha = window[kCGWindowAlpha as String] as? Double, alpha == 0 { continue }
        if (window[kCGWindowNumber as String] as? CGWindowID) == target { return true }
        let bounds = window[kCGWindowBounds as String] as? [String: Any] ?? [:]
        let area = ((bounds["Width"] as? Double) ?? .infinity) * ((bounds["Height"] as? Double) ?? .infinity)
        let sameApp = (window[kCGWindowOwnerPID as String] as? pid_t) == pid
        if sameApp && area < 240_000 { continue }
        return false
    }
    return false
}

func topWindowOwner() -> String {
    let options: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
    guard let list = CGWindowListCopyWindowInfo(options, kCGNullWindowID) as? [[String: Any]] else { return "unknown" }
    for window in list where (window[kCGWindowLayer as String] as? Int) == 0 {
        if let alpha = window[kCGWindowAlpha as String] as? Double, alpha == 0 { continue }
        let owner = window[kCGWindowOwnerName as String] as? String ?? "unknown"
        let number = window[kCGWindowNumber as String] as? CGWindowID ?? 0
        return "\(owner) id=\(number)"
    }
    return "none"
}

func focusWindow(_ id: CGWindowID) {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/bin/env")
    process.arguments = ["aerospace", "focus", "--window-id", String(id)]
    try? process.run()
    process.waitUntilExit()
}

func runAppleScript(_ source: String) {
    let process = Process()
    process.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
    process.arguments = ["-e", source]
    try? process.run()
    process.waitUntilExit()
}

func setFront(_ pid: pid_t) {
    runAppleScript("tell application \"System Events\" to set frontmost of (first process whose unix id is \(pid)) to true")
}

final class FrameWatcher: NSObject, SCStreamOutput, SCStreamDelegate {
    private let lock = NSLock()
    private var baseline = [UInt8]()
    private var armedAt: UInt64?
    private var signal: DispatchSemaphore?
    private var displayDelta: UInt64?
    private var arrivalDelta: UInt64?
    private(set) var changedFrames = 0
    private(set) var staleFrames = 0
    private(set) var streamError: String?
    private var bytesPerRow = 0
    private var region: (left: Int, top: Int, right: Int, bottom: Int)?

    func setRegion(_ area: (left: Int, top: Int, right: Int, bottom: Int)) {
        lock.lock()
        region = area
        lock.unlock()
    }

    /// Latest whole frame, valid while no region is set, because the baseline then follows every change.
    func snapshot() -> (pixels: [UInt8], bytesPerRow: Int) {
        lock.lock()
        defer { lock.unlock() }
        return (baseline, bytesPerRow)
    }

    func stream(_ stream: SCStream, didStopWithError error: Error) {
        streamError = error.localizedDescription
    }

    func stream(_ stream: SCStream, didOutputSampleBuffer sample: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .screen, sample.isValid,
            let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: false) as? [[SCStreamFrameInfo: Any]],
            let info = attachments.first,
            let rawStatus = info[.status] as? Int,
            SCFrameStatus(rawValue: rawStatus) == .complete,
            let pixels = CMSampleBufferGetImageBuffer(sample)
        else { return }
        let arrival = mach_absolute_time()
        let display = (info[.displayTime] as? UInt64) ?? arrival

        CVPixelBufferLockBaseAddress(pixels, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(pixels, .readOnly) }
        guard let base = CVPixelBufferGetBaseAddress(pixels) else { return }
        let rowBytes = CVPixelBufferGetBytesPerRow(pixels)
        let size = rowBytes * CVPixelBufferGetHeight(pixels)

        lock.lock()
        defer { lock.unlock() }
        bytesPerRow = rowBytes
        if baseline.count != size {
            baseline = [UInt8](UnsafeRawBufferPointer(start: base, count: size))
            return
        }
        let same: Bool
        if let area = region {
            same = baseline.withUnsafeBytes { old in
                for row in area.top...area.bottom {
                    let offset = row * rowBytes + area.left * 4
                    let length = (area.right - area.left + 1) * 4
                    if memcmp(old.baseAddress! + offset, base + offset, length) != 0 { return false }
                }
                return true
            }
        } else {
            same = baseline.withUnsafeBytes { memcmp($0.baseAddress!, base, size) == 0 }
        }
        if same { return }
        baseline.withUnsafeMutableBytes { _ = memcpy($0.baseAddress!, base, size) }
        changedFrames += 1

        guard let armed = armedAt else { return }
        if display < armed {
            staleFrames += 1
            return
        }
        displayDelta = display - armed
        arrivalDelta = arrival - armed
        armedAt = nil
        signal?.signal()
    }

    /// Returns (ms to the frame's display time, ms until the frame reached this tool), or nil on timeout.
    func press(_ key: CGKeyCode, source: CGEventSource?) -> (Double, Double)? {
        let wait = DispatchSemaphore(value: 0)
        guard let down = CGEvent(keyboardEventSource: source, virtualKey: key, keyDown: true),
            let up = CGEvent(keyboardEventSource: source, virtualKey: key, keyDown: false)
        else { return nil }
        lock.lock()
        signal = wait
        displayDelta = nil
        arrivalDelta = nil
        armedAt = mach_absolute_time()
        lock.unlock()
        down.post(tap: .cghidEventTap)
        up.post(tap: .cghidEventTap)
        let finished = wait.wait(timeout: .now() + 1.0) == .success
        lock.lock()
        defer { lock.unlock() }
        armedAt = nil
        guard finished, let shown = displayDelta, let arrived = arrivalDelta else { return nil }
        return (machMilliseconds(shown), machMilliseconds(arrived))
    }
}

func tap(_ key: CGKeyCode, source: CGEventSource?) {
    CGEvent(keyboardEventSource: source, virtualKey: key, keyDown: true)?.post(tap: .cghidEventTap)
    CGEvent(keyboardEventSource: source, virtualKey: key, keyDown: false)?.post(tap: .cghidEventTap)
}

/// Finds the pixels that change on every key press, so other activity in the window cannot be mistaken for the echo.
func findEchoRegion(_ watcher: FrameWatcher, _ target: CGWindowID, pid: pid_t, source: CGEventSource?) -> (left: Int, top: Int, right: Int, bottom: Int)? {
    var mask = [Bool]()
    var rowBytes = 0
    for press in 0..<8 {
        guard targetIsOnTop(target, pid: pid) else {
            print("stopped during calibration: on top is \(topWindowOwner())")
            return nil
        }
        let before = watcher.snapshot()
        tap(press % 2 == 0 ? periodKey : deleteKey, source: source)
        Thread.sleep(forTimeInterval: 0.25)
        let after = watcher.snapshot()
        guard before.pixels.count == after.pixels.count, before.bytesPerRow == after.bytesPerRow, before.bytesPerRow > 0 else { return nil }
        rowBytes = before.bytesPerRow
        let pixelCount = before.pixels.count / 4
        if mask.isEmpty { mask = [Bool](repeating: true, count: pixelCount) }
        before.pixels.withUnsafeBytes { (old: UnsafeRawBufferPointer) in
            after.pixels.withUnsafeBytes { (new: UnsafeRawBufferPointer) in
                let a = old.bindMemory(to: UInt32.self)
                let b = new.bindMemory(to: UInt32.self)
                for i in 0..<pixelCount where mask[i] && a[i] == b[i] { mask[i] = false }
            }
        }
        // With no key pressed, anything that still changes is unrelated animation, so it can't be the echo.
        let idleStart = watcher.snapshot()
        Thread.sleep(forTimeInterval: 0.25)
        let idleEnd = watcher.snapshot()
        guard idleStart.pixels.count == idleEnd.pixels.count else { return nil }
        idleStart.pixels.withUnsafeBytes { (old: UnsafeRawBufferPointer) in
            idleEnd.pixels.withUnsafeBytes { (new: UnsafeRawBufferPointer) in
                let a = old.bindMemory(to: UInt32.self)
                let b = new.bindMemory(to: UInt32.self)
                for i in 0..<pixelCount where mask[i] && a[i] != b[i] { mask[i] = false }
            }
        }
        let width = before.bytesPerRow / 4
        var changed = 0, left = Int.max, top = Int.max, right = -1, bottom = -1
        before.pixels.withUnsafeBytes { (old: UnsafeRawBufferPointer) in
            after.pixels.withUnsafeBytes { (new: UnsafeRawBufferPointer) in
                let a = old.bindMemory(to: UInt32.self)
                let b = new.bindMemory(to: UInt32.self)
                for i in 0..<pixelCount where a[i] != b[i] {
                    changed += 1
                    let x = i % width, y = i / width
                    left = min(left, x); right = max(right, x); top = min(top, y); bottom = max(bottom, y)
                }
            }
        }
        print("calibration press \(press): changed=\(changed) box x=\(left)-\(right) y=\(top)-\(bottom) kept=\(mask.filter { $0 }.count)")
    }
    let width = rowBytes / 4
    var left = Int.max, top = Int.max, right = -1, bottom = -1
    for i in 0..<mask.count where mask[i] {
        let x = i % width, y = i / width
        left = min(left, x); right = max(right, x); top = min(top, y); bottom = max(bottom, y)
    }
    guard right >= 0 else {
        print("calibration found no pixels that change on every key")
        return nil
    }
    guard right - left < 400, bottom - top < 120 else {
        print("calibration area too large: \(right - left + 1)x\(bottom - top + 1) pixels")
        return nil
    }
    return (left, top, right, bottom)
}

func describe(_ label: String, _ values: [Double]) {
    guard !values.isEmpty else {
        print("\(label): no samples")
        return
    }
    let sorted = values.sorted()
    func at(_ fraction: Double) -> Double { sorted[min(sorted.count - 1, Int(Double(sorted.count) * fraction))] }
    print(String(format: "%@: samples=%d min=%.1f median=%.1f p95=%.1f max=%.1f ms", label, sorted.count, sorted[0], at(0.5), at(0.95), sorted[sorted.count - 1]))
}

@main
struct KeyLat {
    static func main() async {
        let args = CommandLine.arguments
        guard (4...6).contains(args.count), let rawPid = Int32(args[1]), let rounds = Int(args[2]) else {
            print("usage: keylat <pid> <rounds> <raw csv path> [window id] [keep-size | keep-size-calibrate]")
            exit(2)
        }
        let wantedWindow = args.count >= 5 ? CGWindowID(args[4]) : nil
        let mode = args.count == 6 ? args[5] : ""
        let keepSize = mode.hasPrefix("keep-size")
        let calibrate = mode == "keep-size-calibrate"
        let previous = NSWorkspace.shared.frontmostApplication?.processIdentifier
        print("accessibility=\(AXIsProcessTrusted()) screen=\(CGPreflightScreenCaptureAccess())")
        print("display max fps=\(NSScreen.main?.maximumFramesPerSecond ?? 0)")

        if !keepSize {
            runAppleScript("tell application \"System Events\" to tell (first process whose unix id is \(rawPid)) to tell window 1\nset position to {60, 60}\nset size to {1200, 800}\nend tell")
        }
        setFront(rawPid)
        if let wantedWindow { focusWindow(wantedWindow) }
        try? await Task.sleep(for: .milliseconds(500))

        do {
            let content = try await SCShareableContent.excludingDesktopWindows(true, onScreenWindowsOnly: true)
            let owned = content.windows.filter { $0.owningApplication?.processID == rawPid && $0.windowLayer == 0 }
            let chosen = wantedWindow.map { id in owned.first { $0.windowID == id } } ?? owned.max(by: { $0.frame.width * $0.frame.height < $1.frame.width * $1.frame.height })
            guard let window = chosen else {
                print("no window found for pid \(rawPid)")
                exit(1)
            }
            print("target \(window.owningApplication?.applicationName ?? "?") window \(Int(window.frame.width))x\(Int(window.frame.height)) id=\(window.windowID)")

            let filter = SCContentFilter(desktopIndependentWindow: window)
            let config = SCStreamConfiguration()
            let scale = CGFloat(filter.pointPixelScale)
            config.width = Int(filter.contentRect.width * scale)
            config.height = Int(filter.contentRect.height * scale)
            config.minimumFrameInterval = CMTime(value: 1, timescale: 120)
            config.pixelFormat = kCVPixelFormatType_32BGRA
            config.queueDepth = 5
            config.showsCursor = false

            let watcher = FrameWatcher()
            let stream = SCStream(filter: filter, configuration: config, delegate: watcher)
            try stream.addStreamOutput(watcher, type: .screen, sampleHandlerQueue: DispatchQueue(label: "frames", qos: .userInteractive))
            try await stream.startCapture()
            try await Task.sleep(for: .seconds(1))
            let idleStart = watcher.changedFrames
            try await Task.sleep(for: .seconds(1))
            print("changes while idle for 1 s=\(watcher.changedFrames - idleStart)")

            var typed = [Double]()
            var erased = [Double]()
            var typedArrival = [Double]()
            var missed = 0
            var csv = "round,typed_display_ms,typed_arrival_ms,erased_display_ms,erased_arrival_ms\n"
            var stoppedEarly = false

            await withCheckedContinuation { (done: CheckedContinuation<Void, Never>) in
                DispatchQueue.global(qos: .userInteractive).async {
                    let source = CGEventSource(stateID: .hidSystemState)
                    if calibrate {
                        guard let area = findEchoRegion(watcher, window.windowID, pid: rawPid, source: source) else {
                            stoppedEarly = true
                            done.resume()
                            return
                        }
                        print("echo area x=\(area.left) y=\(area.top) w=\(area.right - area.left + 1) h=\(area.bottom - area.top + 1) pixels")
                        watcher.setRegion(area)
                        Thread.sleep(forTimeInterval: 0.3)
                        let before = watcher.changedFrames
                        Thread.sleep(forTimeInterval: 2)
                        print("changes in echo area while idle for 2 s=\(watcher.changedFrames - before)")
                    }
                    for round in 0..<rounds {
                        guard targetIsOnTop(window.windowID, pid: rawPid) else {
                            print("stopped at round \(round): on top is \(topWindowOwner()), wanted id=\(window.windowID)")
                            stoppedEarly = true
                            break
                        }
                        let one = watcher.press(periodKey, source: source)
                        Thread.sleep(forTimeInterval: Double.random(in: 0.08...0.2))
                        guard targetIsOnTop(window.windowID, pid: rawPid) else {
                            print("stopped at round \(round): on top is \(topWindowOwner()), wanted id=\(window.windowID)")
                            stoppedEarly = true
                            break
                        }
                        let two = watcher.press(deleteKey, source: source)
                        Thread.sleep(forTimeInterval: Double.random(in: 0.08...0.2))
                        if let one {
                            typed.append(one.0)
                            typedArrival.append(one.1)
                        } else {
                            missed += 1
                        }
                        if let two { erased.append(two.0) } else { missed += 1 }
                        csv += "\(round),\(one?.0 ?? -1),\(one?.1 ?? -1),\(two?.0 ?? -1),\(two?.1 ?? -1)\n"
                    }
                    done.resume()
                }
            }

            try await stream.stopCapture()
            try csv.write(toFile: args[3], atomically: true, encoding: .utf8)
            describe("key typed, frame display time", typed)
            describe("key typed, frame reached tool", typedArrival)
            describe("backspace, frame display time", erased)
            print("missed=\(missed) frames shown before the key=\(watcher.staleFrames) stopped early=\(stoppedEarly) stream error=\(watcher.streamError ?? "none")")
        } catch {
            print("failed: \(error.localizedDescription)")
        }
        if let previous, previous != rawPid { setFront(previous) }
        print("finished")
        exit(0)
    }
}
