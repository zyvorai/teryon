// TeryonMenu: a menu-bar item for TERYON. It shows how many AI agents need you, lists every agent
// with its state, and a click on one brings TERYON to that workspace and pane.
//
// It only runs `teryon agents --all --json` and `teryon notify open ...`; it holds no state of its own.
import AppKit
import Foundation
import ServiceManagement
import UserNotifications

struct Agent {
    let host: String, workspace: String, pane: Int, name: String, status: String
}

/// The teryon program next to this app (TERYON.app/Contents/MacOS/teryon), or an installed one.
func teryonPath() -> String? {
    let here = Bundle.main.bundleURL  // .../TERYON.app/Contents/Library/TeryonMenu.app
    let beside = here.deletingLastPathComponent().deletingLastPathComponent()
        .appendingPathComponent("MacOS/teryon").path
    let home = NSHomeDirectory()
    return [ProcessInfo.processInfo.environment["TERYON_BIN"] ?? "", beside, "/Applications/TERYON.app/Contents/MacOS/teryon", "\(home)/.local/bin/teryon", "/usr/local/bin/teryon", "/opt/homebrew/bin/teryon"]
        .first { FileManager.default.isExecutableFile(atPath: $0) }
}

func run(_ bin: String, _ args: [String], timeout: TimeInterval? = nil) -> String? {
    let p = Process()
    p.executableURL = URL(fileURLWithPath: bin)
    p.arguments = args
    var env = ProcessInfo.processInfo.environment
    env["TERYON_MENU_RUNNING"] = "1"   // lets `teryon doctor --mac` know this app is up
    p.environment = env
    let out = Pipe()
    p.standardOutput = out
    p.standardError = FileHandle.nullDevice
    do { try p.run() } catch { return nil }
    if let timeout = timeout {
        // A host that does not answer must not hold the menu hostage.
        DispatchQueue.global().asyncAfter(deadline: .now() + timeout) { if p.isRunning { p.terminate() } }
    }
    let data = out.fileHandleForReading.readDataToEndOfFile()
    p.waitUntilExit()
    return String(data: data, encoding: .utf8)
}

/// One JSON object per line, as `teryon agents --json` prints them.
func parse(_ text: String) -> [Agent] {
    text.split(separator: "\n").compactMap { line in
        guard let d = line.data(using: .utf8),
              let o = try? JSONSerialization.jsonObject(with: d) as? [String: Any],
              let host = o["host"] as? String, let ws = o["workspace"] as? String,
              let pane = o["pane"] as? Int, let name = o["agent"] as? String,
              let status = o["status"] as? String else { return nil }
        return Agent(host: host, workspace: ws, pane: pane, name: name, status: status)
    }
}

struct Host { let name: String, up: Bool, load: Double?, mem: Int?, thermal: String?, latency: Int? }

func parseHosts(_ text: String) -> [Host] {
    text.split(separator: "\n").compactMap { line in
        guard let d = line.data(using: .utf8),
              let o = try? JSONSerialization.jsonObject(with: d) as? [String: Any],
              let name = o["host"] as? String, let up = o["up"] as? Bool else { return nil }
        return Host(name: name, up: up, load: o["load1"] as? Double, mem: o["mem_used_pct"] as? Int,
                    thermal: o["thermal"] as? String, latency: o["latency_ms"] as? Int)
    }
}

/// "mini · load 2.1 · mem 59% · 11 ms", or "mini · not answering".
func describe(_ h: Host) -> String {
    guard h.up else { return "\(h.name) · not answering" }
    var parts = [h.name]
    if let l = h.load { parts.append(String(format: "load %.1f", l)) }
    if let m = h.mem { parts.append("mem \(m)%") }
    if let t = h.thermal, t != "nominal" { parts.append(t) }
    if let ms = h.latency, h.name != "local" { parts.append("\(ms) ms") }
    return parts.joined(separator: " · ")
}

struct Recent { let host: String, workspace: String }

func parseRecent(_ text: String) -> [Recent] {
    text.split(separator: "\n").compactMap { line in
        guard let d = line.data(using: .utf8),
              let o = try? JSONSerialization.jsonObject(with: d) as? [String: Any],
              let host = o["host"] as? String, let ws = o["workspace"] as? String else { return nil }
        return Recent(host: host, workspace: ws)
    }
}

final class Controller: NSObject, NSMenuDelegate, UNUserNotificationCenterDelegate {
    let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
    var agents: [Agent] = []
    var recents: [Recent] = []
    var hosts: [Host] = []
    var probing = false
    /// How many `doctor --mac` checks fail; nil until the first run.
    var setupProblems: Int?
    /// Agents (host/workspace/pane) already announced as needing you, so each prompt is announced once.
    var announced = Set<String>()
    var notifyOn: Bool {
        get { UserDefaults.standard.bool(forKey: "notifyNeedsYou") }
        set { UserDefaults.standard.set(newValue, forKey: "notifyNeedsYou") }
    }
    var reachable = true
    let bin = teryonPath()

    override init() {
        super.init()
        let center = UNUserNotificationCenter.current()
        center.delegate = self
        center.setNotificationCategories([UNNotificationCategory(
            identifier: "asking",
            actions: [UNNotificationAction(identifier: "approve", title: "Approve", options: [.authenticationRequired]),
                      UNNotificationAction(identifier: "deny", title: "Deny", options: [.destructive])],
            intentIdentifiers: [], options: [])])
        refresh()
        Timer.scheduledTimer(withTimeInterval: 3, repeats: true) { [weak self] _ in self?.refresh() }
        probeHosts()
        checkSetupQuietly()
        Timer.scheduledTimer(withTimeInterval: 120, repeats: true) { [weak self] _ in self?.checkSetupQuietly() }
        Timer.scheduledTimer(withTimeInterval: 20, repeats: true) { [weak self] _ in self?.probeHosts() }
    }

    func refresh() {
        guard let bin = bin else { reachable = false; render(); return }
        DispatchQueue.global(qos: .utility).async {
            let text = run(bin, ["agents", "--all", "--json"])
            let recent = run(bin, ["recent", "--limit", "8", "--json"])
            DispatchQueue.main.async {
                self.reachable = text != nil
                self.agents = text.map(parse) ?? []
                self.announce()
                self.recents = recent.map(parseRecent) ?? []
                self.render()
            }
        }
    }

    /// Post a native notification for each agent that has just started needing you, and take back the
    /// ones whose agent has moved on. This app stays running, so it receives the button taps itself.
    func announce() {
        let center = UNUserNotificationCenter.current()
        let waiting = Set(agents.filter { $0.status == "needs you" }.map { "\($0.host)/\($0.workspace)/\($0.pane)" })
        let gone = announced.subtracting(waiting)
        if !gone.isEmpty { center.removeDeliveredNotifications(withIdentifiers: Array(gone)) }
        guard notifyOn else { announced = waiting; return }
        for a in agents where a.status == "needs you" {
            let id = "\(a.host)/\(a.workspace)/\(a.pane)"
            if announced.contains(id) { continue }
            let c = UNMutableNotificationContent()
            c.title = "\(a.name) needs you"
            c.subtitle = "\(a.workspace) · pane \(a.pane)"
            c.body = a.host == "local" ? "Waiting for your answer" : "Waiting for your answer on \(a.host)"
            c.sound = UNNotificationSound(named: UNNotificationSoundName("Glass"))
            c.threadIdentifier = "\(a.host)/\(a.workspace)"
            c.categoryIdentifier = "asking"
            c.interruptionLevel = .timeSensitive
            c.userInfo = ["host": a.host, "workspace": a.workspace, "pane": a.pane]
            center.add(UNNotificationRequest(identifier: id, content: c, trigger: nil))
        }
        announced = waiting
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, didReceive response: UNNotificationResponse,
                                withCompletionHandler done: @escaping () -> Void) {
        let i = response.notification.request.content.userInfo
        if let bin = bin, let host = i["host"] as? String, let ws = i["workspace"] as? String, let pane = i["pane"] as? Int {
            switch response.actionIdentifier {
            case "approve", "deny":
                var args = ["respond", host, "-w", ws, "--pane", String(pane)]
                if response.actionIdentifier == "deny" { args.append("--deny") }
                DispatchQueue.global().async { _ = run(bin, args); DispatchQueue.main.async { self.refresh() } }
            case UNNotificationDefaultActionIdentifier:
                DispatchQueue.global().async { _ = run(bin, ["notify", "open", host, ws, String(pane)]) }
            default: break
            }
        }
        done()
    }

    func userNotificationCenter(_ center: UNUserNotificationCenter, willPresent n: UNNotification,
                                withCompletionHandler done: @escaping (UNNotificationPresentationOptions) -> Void) {
        done([.banner, .sound, .list])
    }

    /// Run `teryon doctor --mac` and show what it found.
    /// Count what `doctor --mac` would flag, so the menu can say so without opening anything.
    func checkSetupQuietly() {
        guard let bin = bin else { return }
        DispatchQueue.global(qos: .utility).async {
            let text = run(bin, ["doctor", "--mac"], timeout: 20)
            let bad = text.map { $0.split(separator: "\n").filter { $0.hasPrefix("✗") }.count }
            DispatchQueue.main.async { self.setupProblems = bad; self.render() }
        }
    }

    @objc func checkSetup() {
        guard let bin = bin else { return }
        DispatchQueue.global().async {
            let text = run(bin, ["doctor", "--mac"], timeout: 20) ?? "Could not run teryon doctor."
            DispatchQueue.main.async {
                let alert = NSAlert()
                alert.messageText = "TERYON setup"
                alert.informativeText = text
                alert.addButton(withTitle: "OK")
                NSApp.activate(ignoringOtherApps: true)
                alert.runModal()
            }
        }
    }

    @objc func toggleNotify() {
        if notifyOn { notifyOn = false; render(); return }
        UNUserNotificationCenter.current().requestAuthorization(options: [.alert, .sound]) { granted, _ in
            DispatchQueue.main.async { self.notifyOn = granted; self.render() }
        }
    }

    /// Host health is slower (it may reach other machines), so it has its own timer and never overlaps.
    func probeHosts() {
        guard let bin = bin, !probing else { return }
        probing = true
        DispatchQueue.global(qos: .utility).async {
            let text = run(bin, ["dashboard", "--local", "--json"], timeout: 25)
            DispatchQueue.main.async {
                self.probing = false
                if let text = text { self.hosts = parseHosts(text); self.render() }
            }
        }
    }

    func render() {
        let needs = agents.filter { $0.status == "needs you" }.count
        let working = agents.filter { $0.status == "working" }.count
        let button = item.button
        let symbol = needs > 0 ? "exclamationmark.bubble.fill" : (working > 0 ? "terminal.fill" : "terminal")
        button?.image = NSImage(systemSymbolName: symbol, accessibilityDescription: "TERYON")
        button?.title = needs > 0 ? " \(needs)" : ""
        button?.toolTip = needs > 0 ? "\(needs) agent\(needs == 1 ? "" : "s") need you" : "TERYON"
        let menu = NSMenu()
        if !reachable {
            menu.addItem(withTitle: "TERYON not found", action: nil, keyEquivalent: "")
        } else if agents.isEmpty {
            menu.addItem(withTitle: "No agents running", action: nil, keyEquivalent: "")
        } else {
            // Agents that need you first, then working, then the rest.
            let rank = ["needs you": 0, "working": 1, "idle": 2, "done": 3]
            for a in agents.sorted(by: { (rank[$0.status] ?? 9) < (rank[$1.status] ?? 9) }) {
                let mark = a.status == "needs you" ? "●  " : (a.status == "working" ? "◐  " : "○  ")
                let m = NSMenuItem(title: "\(mark)\(a.name) · \(a.workspace) · pane \(a.pane) — \(a.status)",
                                   action: #selector(open(_:)), keyEquivalent: "")
                m.target = self
                m.representedObject = a
                menu.addItem(m)
                // An agent that needs you can be answered right here; the daemon types nothing if it
                // has moved on since the menu was drawn.
                if a.status == "needs you" {
                    for (title, deny) in [("Approve", false), ("Deny", true)] {
                        let r = NSMenuItem(title: title, action: #selector(respond(_:)), keyEquivalent: "")
                        r.target = self
                        r.representedObject = a
                        r.tag = deny ? 1 : 0
                        r.indentationLevel = 2
                        menu.addItem(r)
                    }
                }
            }
        }
        if !hosts.isEmpty {
            menu.addItem(.separator())
            for h in hosts {
                let m = NSMenuItem(title: "\(h.up ? "●" : "○")  \(describe(h))", action: nil, keyEquivalent: "")
                m.isEnabled = false
                menu.addItem(m)
            }
        }
        if !recents.isEmpty {
            menu.addItem(.separator())
            let sub = NSMenu()
            for r in recents {
                let label = r.host == "local" ? r.workspace : "\(r.workspace) — \(r.host)"
                let m = NSMenuItem(title: label, action: #selector(openRecent(_:)), keyEquivalent: "")
                m.target = self
                m.representedObject = r
                sub.addItem(m)
            }
            let parent = NSMenuItem(title: "Recent Workspaces", action: nil, keyEquivalent: "")
            parent.submenu = sub
            menu.addItem(parent)
        }
        menu.addItem(.separator())
        let show = NSMenuItem(title: "Open TERYON", action: #selector(openApp), keyEquivalent: "")
        show.target = self
        menu.addItem(show)
        let check = NSMenuItem(title: "Check Setup…", action: #selector(checkSetup), keyEquivalent: "")
        check.target = self
        if let n = setupProblems, n > 0 { check.title = "Check Setup… (\(n) to fix)" }
        menu.addItem(check)
        let notify = NSMenuItem(title: "Notify When an Agent Needs Me", action: #selector(toggleNotify), keyEquivalent: "")
        notify.target = self
        notify.state = notifyOn ? .on : .off
        menu.addItem(notify)
        let login = NSMenuItem(title: "Launch at Login", action: #selector(toggleLogin), keyEquivalent: "")
        login.target = self
        switch SMAppService.mainApp.status {
        case .enabled: login.state = .on
        case .requiresApproval: login.title = "Launch at Login (allow in System Settings)"; login.state = .mixed
        default: login.state = .off
        }
        menu.addItem(login)
        let quit = NSMenuItem(title: "Quit Menu Item", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
        menu.addItem(quit)
        item.menu = menu
    }

    @objc func open(_ sender: NSMenuItem) {
        guard let a = sender.representedObject as? Agent, let bin = bin else { return }
        DispatchQueue.global().async { _ = run(bin, ["notify", "open", a.host, a.workspace, String(a.pane)]) }
    }

    @objc func openRecent(_ sender: NSMenuItem) {
        guard let r = sender.representedObject as? Recent, let bin = bin else { return }
        DispatchQueue.global().async { _ = run(bin, ["notify", "open", r.host, r.workspace, "0"]) }
    }

    @objc func respond(_ sender: NSMenuItem) {
        guard let a = sender.representedObject as? Agent, let bin = bin else { return }
        var args = ["respond", a.host, "-w", a.workspace, "--pane", String(a.pane)]
        if sender.tag == 1 { args.append("--deny") }
        DispatchQueue.global().async {
            _ = run(bin, args)
            DispatchQueue.main.async { self.refresh() }
        }
    }

    @objc func toggleLogin() {
        let service = SMAppService.mainApp
        do {
            if service.status == .enabled || service.status == .requiresApproval {
                try service.unregister()
            } else {
                try service.register()
                // macOS may want the person to allow it once in Login Items.
                if service.status == .requiresApproval { SMAppService.openSystemSettingsLoginItems() }
            }
        } catch {
            NSLog("TeryonMenu: launch at login: \(error)")
        }
        render()
    }

    @objc func openApp() {
        let outer = Bundle.main.bundleURL.deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        NSWorkspace.shared.open(outer.pathExtension == "app" ? outer : URL(fileURLWithPath: "/Applications/TERYON.app"))
    }
}

/// `teryon://open/<host>/<workspace>[/<pane>]` brings TERYON to that workspace. Shortcuts, Siri and
/// Spotlight can open such a link ("Open URL"), so "open my build workspace" needs no script.
/// Only this one fixed verb is understood and the names are checked, so a link cannot run anything else.
func handle(url: URL, bin: String?) {
    guard url.scheme == "teryon", url.host == "open", let bin = bin else { return }
    let parts = url.pathComponents.filter { $0 != "/" }
    guard parts.count == 2 || parts.count == 3 else { return }
    let ok = { (s: String) in !s.isEmpty && s.count <= 64 && s.allSatisfy { $0.isASCII && ($0.isLetter || $0.isNumber || "-_.".contains($0)) } }
    guard ok(parts[0]), ok(parts[1]), !parts[0].hasPrefix("."), !parts[1].hasPrefix(".") else { return }
    var pane: UInt64 = 0
    if parts.count == 3 {
        guard let p = UInt64(parts[2]) else { return }
        pane = p
    }
    DispatchQueue.global().async { _ = run(bin, ["notify", "open", parts[0], parts[1], String(pane)]) }
}

final class LinkHandler: NSObject {
    let bin = teryonPath()
    @objc func get(_ event: NSAppleEventDescriptor, reply: NSAppleEventDescriptor) {
        if let s = event.paramDescriptor(forKeyword: keyDirectObject)?.stringValue, let url = URL(string: s) {
            handle(url: url, bin: bin)
        }
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let links = LinkHandler()
NSAppleEventManager.shared().setEventHandler(links, andSelector: #selector(LinkHandler.get(_:reply:)),
                                              forEventClass: AEEventClass(kInternetEventClass), andEventID: AEEventID(kAEGetURL))
let controller = Controller()
app.run()
