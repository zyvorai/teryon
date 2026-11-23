// TeryonNotifier: posts native macOS notifications for TERYON and handles their buttons.
//
//   post mode  (run by teryon, JSON on stdin): add the notification, exit 0; exit 2 if the user has
//              turned TERYON notifications off; any other code means "could not post".
//   reply mode (launched by macOS when a notification or one of its buttons is used): run
//              `<bin> notify answer approve:<token>` or `<bin> notify open <workspace> <pane>`, then exit.
//
// The program to run comes from the notification itself, so nothing is read from the environment.
import AppKit
import Foundation
import UserNotifications

struct Payload: Decodable {
    var title: String
    var subtitle: String
    var body: String
    var sound: String?
    var thread: String
    var host: String
    var workspace: String
    var pane: UInt64
    var level: String
    var token: String?
    var bin: String
    var runtime: String?
}

let askingCategory = "teryon.asking"
let plainCategory = "teryon.plain"

func registerCategories(_ center: UNUserNotificationCenter) {
    let approve = UNNotificationAction(identifier: "approve", title: "Approve", options: [.authenticationRequired])
    let deny = UNNotificationAction(identifier: "deny", title: "Deny", options: [.destructive])
    center.setNotificationCategories([
        UNNotificationCategory(identifier: askingCategory, actions: [approve, deny], intentIdentifiers: [], options: []),
        UNNotificationCategory(identifier: plainCategory, actions: [], intentIdentifiers: [], options: []),
    ])
}

func run(_ bin: String, _ args: [String], runtime: String?) {
    let p = Process()
    if let runtime = runtime, !runtime.isEmpty {
        var env = ProcessInfo.processInfo.environment
        env["TERYON_RUNTIME_DIR"] = runtime
        p.environment = env
    }
    p.executableURL = URL(fileURLWithPath: bin)
    p.arguments = args
    p.standardOutput = FileHandle.nullDevice
    p.standardError = FileHandle.nullDevice
    do { try p.run(); p.waitUntilExit() } catch {}
}

final class Responder: NSObject, UNUserNotificationCenterDelegate {
    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse,
                                withCompletionHandler done: @escaping () -> Void) {
        let info = response.notification.request.content.userInfo
        let bin = info["bin"] as? String ?? ""
        let token = info["token"] as? String
        let runtime = info["runtime"] as? String
        // Only the program that posted the notification is run, and only with these fixed verbs.
        if !bin.isEmpty, FileManager.default.isExecutableFile(atPath: bin) {
            switch response.actionIdentifier {
            case "approve", "deny":
                if let token = token { run(bin, ["notify", "answer", "\(response.actionIdentifier):\(token)"], runtime: runtime) }
            case UNNotificationDefaultActionIdentifier:
                let host = info["host"] as? String ?? "local"
                let ws = info["workspace"] as? String ?? ""
                let pane = info["pane"] as? UInt64 ?? 0
                run(bin, ["notify", "open", host, ws, String(pane)], runtime: runtime)
            default:
                break
            }
        }
        done()
        exit(0)
    }
    // Show the banner even if TERYON were frontmost; the daemon already decides when to notify.
    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent n: UNNotification,
                                withCompletionHandler done: @escaping (UNNotificationPresentationOptions) -> Void) {
        done([.banner, .sound, .list])
    }
}

let responder = Responder()
let center = UNUserNotificationCenter.current()
center.delegate = responder
registerCategories(center)

let mode = CommandLine.arguments.dropFirst().first
if mode == "status" {
    // What macOS currently lets TERYON do, as JSON on stdout (`teryon notify status` reads it).
    let gate = DispatchSemaphore(value: 0)
    center.getNotificationSettings { s in
        func name(_ v: UNNotificationSetting) -> String { v == .enabled ? "enabled" : (v == .disabled ? "disabled" : "unsupported") }
        let auth: String
        switch s.authorizationStatus {
        case .authorized: auth = "authorized"
        case .denied: auth = "denied"
        case .provisional: auth = "provisional"
        case .ephemeral: auth = "ephemeral"
        default: auth = "not_determined"
        }
        let out: [String: Any] = ["authorization": auth, "alert": name(s.alertSetting), "sound": name(s.soundSetting),
                                  "notification_center": name(s.notificationCenterSetting),
                                  "time_sensitive": name(s.timeSensitiveSetting)]
        if let d = try? JSONSerialization.data(withJSONObject: out, options: [.sortedKeys]) {
            print(String(data: d, encoding: .utf8) ?? "{}")
        }
        gate.signal()
    }
    _ = gate.wait(timeout: .now() + 5)
    exit(0)
}
if mode == "post" || mode == "post-file" {
    // post-file is for testing through LaunchServices (`open -W -a`), which cannot pass stdin.
    let raw = mode == "post-file" && CommandLine.arguments.count > 2
        ? FileManager.default.contents(atPath: CommandLine.arguments[2]) ?? Data()
        : FileHandle.standardInput.readDataToEndOfFile()
    guard let payload = try? JSONDecoder().decode(Payload.self, from: raw) else {
        exit(64)
    }
    // A real (windowless) app must be running for macOS to show the permission prompt and deliver
    // the answer, so the work happens in callbacks and the main run loop stays free.
    let app = NSApplication.shared
    app.setActivationPolicy(.accessory)
    func finish(_ code: Int32) { exit(code) }
    center.requestAuthorization(options: [.alert, .sound]) { granted, error in
        if let error = error { FileHandle.standardError.write(Data("teryon-notifier: \(error)\n".utf8)) }
        guard granted else {
            // Only an explicit "no" in System Settings ends the matter; if the question could not be
            // asked (no GUI session) the caller falls back to the AppleScript banner.
            center.getNotificationSettings { settings in
                FileHandle.standardError.write(Data("teryon-notifier: authorizationStatus=\(settings.authorizationStatus.rawValue)\n".utf8))
                finish(settings.authorizationStatus == .denied ? 2 : 1)
            }
            return
        }
        let c = UNMutableNotificationContent()
        c.title = payload.title
        c.subtitle = payload.subtitle
        c.body = payload.body
        c.threadIdentifier = payload.thread
        c.categoryIdentifier = payload.token == nil ? plainCategory : askingCategory
        c.userInfo = ["bin": payload.bin, "host": payload.host, "workspace": payload.workspace, "pane": payload.pane,
                      "token": payload.token ?? "", "runtime": payload.runtime ?? ""]
        switch payload.level {
        case "time-sensitive": c.interruptionLevel = .timeSensitive
        case "passive": c.interruptionLevel = .passive
        default: c.interruptionLevel = .active
        }
        if let s = payload.sound, payload.level != "passive" { c.sound = UNNotificationSound(named: UNNotificationSoundName(s)) }
        // One live notification per thread and kind, so a repeat replaces the old banner.
        let id = "\(payload.thread)/\(payload.token == nil ? "info" : "asking")"
        center.add(UNNotificationRequest(identifier: id, content: c, trigger: nil)) { error in
            if let error = error { FileHandle.standardError.write(Data("teryon-notifier: \(error)\n".utf8)) }
            finish(error == nil ? 0 : 1)
        }
    }
    // Long enough for a person to answer the first-run permission prompt.
    DispatchQueue.main.asyncAfter(deadline: .now() + 90) { exit(1) }
    app.run()
}

// Reply mode: macOS started us to deliver a click. Wait for it; give up if none arrives.
let app = NSApplication.shared
app.setActivationPolicy(.accessory)
DispatchQueue.main.asyncAfter(deadline: .now() + 15) { exit(0) }
app.run()
