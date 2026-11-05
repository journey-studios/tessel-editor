// A WKWebView host for the performance page: the same engine the desktop editor embeds (wry
// uses WKWebView on macOS), a `tessel://` scheme like the product's, and nothing else of it.
//
//   swiftc -O wk-host.swift -o wk-host        (webkit.mjs does this and caches the binary)
//   wk-host <plan.json> <out.jsonl>
//
// plan.json: { "page": DIR, "assets": DIR, "bundles": {name: runtime.js path}, "runs": [...] }.
// Runs go one after another, each in a fresh WebView (cold page). The window is small, floating
// and never activated, so it does not take focus from whoever is working. It serves only the
// page directory, the chosen runtime bundle and files under the assets directory.
//
// `bridge` runs measure the native round trip: this host sends `frame.render` with
// evaluateJavaScript and times the page's reply on its IPC channel, with a clock that is not
// limited to 1 ms. That is the floor of the WebView bridge without the editor's UI thread.
import AppKit
import WebKit

let arguments = CommandLine.arguments
guard arguments.count >= 3 else {
    FileHandle.standardError.write("usage: wk-host <plan.json> <out.jsonl>\n".data(using: .utf8)!)
    exit(2)
}
let plan = try! JSONSerialization.jsonObject(with: Data(contentsOf: URL(fileURLWithPath: arguments[1]))) as! [String: Any]
var runs = plan["runs"] as! [[String: Any]]
let pageDir = plan["page"] as! String
let assetsDir = URL(fileURLWithPath: plan["assets"] as! String).standardizedFileURL.path
let bundles = plan["bundles"] as! [String: String]
let outURL = URL(fileURLWithPath: arguments[2])

func append(_ line: String) {
    if !FileManager.default.fileExists(atPath: outURL.path) {
        FileManager.default.createFile(atPath: outURL.path, contents: nil)
    }
    let handle = try! FileHandle(forWritingTo: outURL)
    handle.seekToEndOfFile()
    handle.write((line + "\n").data(using: .utf8)!)
    handle.closeFile()
}

func json(_ value: Any) -> String {
    String(data: try! JSONSerialization.data(withJSONObject: value), encoding: .utf8)!
}

func loadAverage() -> [Double] {
    var load = [Double](repeating: 0, count: 3)
    getloadavg(&load, 3)
    return load
}

func percentile(_ values: [Double], _ share: Double) -> Double {
    let sorted = values.sorted()
    return sorted[min(sorted.count - 1, Int(share * Double(sorted.count)))]
}

var currentBundle = ""

class Scheme: NSObject, WKURLSchemeHandler {
    func webView(_ webView: WKWebView, start task: WKURLSchemeTask) {
        let components = URLComponents(url: task.request.url!, resolvingAgainstBaseURL: false)!
        var path: String?
        switch components.path {
        case "/", "/index.html": path = pageDir + "/index.html"
        case "/runtime.js": path = currentBundle
        case "/asset":
            let requested = URL(fileURLWithPath: components.queryItems?.first { $0.name == "p" }?.value ?? "").standardizedFileURL.path
            // Only files of the measurement's own asset directory.
            if requested.hasPrefix(assetsDir + "/") { path = requested }
        default:
            if components.path.hasPrefix("/perf/"), !components.path.contains("..") {
                path = pageDir + "/" + components.path.dropFirst("/perf/".count)
            }
        }
        guard let file = path, let data = FileManager.default.contents(atPath: file) else {
            task.didFailWithError(NSError(domain: "tessel-perf", code: 404))
            return
        }
        let mime = file.hasSuffix(".html") ? "text/html" : file.hasSuffix(".js") ? "text/javascript" : file.hasSuffix(".png") ? "image/png" : file.hasSuffix(".json") ? "application/json" : "application/octet-stream"
        let response = HTTPURLResponse(
            url: task.request.url!, statusCode: 200, httpVersion: "HTTP/1.1",
            headerFields: ["Content-Type": mime, "Cache-Control": "no-store", "Content-Length": "\(data.count)"])!
        task.didReceive(response)
        task.didReceive(data)
        task.didFinish()
    }

    func webView(_ webView: WKWebView, stop task: WKURLSchemeTask) {}
}

class Host: NSObject, WKScriptMessageHandler {
    var web: WKWebView!
    var window: NSWindow!
    var timer: Timer?
    var run: [String: Any] = [:]
    var sent: [Double] = []
    var received: [Double] = []
    var frameIndex = 0
    var bridgeFrames = 0
    var started = 0.0

    func now() -> Double { Double(DispatchTime.now().uptimeNanoseconds) / 1e6 }

    func next() {
        timer?.invalidate()
        guard !runs.isEmpty else { NSApp.terminate(nil); return }
        run = runs.removeFirst()
        currentBundle = bundles[run["bundle"] as? String ?? "current"]!
        var config = run
        config["assetsDir"] = assetsDir
        let encoded = (try! JSONSerialization.data(withJSONObject: config)).base64EncodedString()
            .addingPercentEncoding(withAllowedCharacters: .alphanumerics)!
        let configuration = WKWebViewConfiguration()
        configuration.setURLSchemeHandler(Scheme(), forURLScheme: "tessel")
        configuration.userContentController.add(self, name: "done")
        configuration.userContentController.add(self, name: "ipc")
        configuration.userContentController.addUserScript(WKUserScript(
            source: "window.__benchDone = (r) => webkit.messageHandlers.done.postMessage(JSON.stringify(r));",
            injectionTime: .atDocumentStart, forMainFrameOnly: true))
        web?.removeFromSuperview()
        web?.configuration.userContentController.removeAllScriptMessageHandlers()
        web = WKWebView(frame: window.contentView!.bounds, configuration: configuration)
        web.autoresizingMask = [.width, .height]
        window.contentView!.addSubview(web)
        sent = []; received = []; frameIndex = 0
        started = now()
        let budget = Double(run["timeout_s"] as? Int ?? 240)
        timer = Timer.scheduledTimer(withTimeInterval: budget, repeats: false) { _ in
            self.finish(["fatal": "timeout after \(budget)s"])
        }
        web.load(URLRequest(url: URL(string: "tessel://localhost/index.html?cfg=\(encoded)")!))
    }

    func finish(_ result: [String: Any]) {
        var record = result
        record["engine"] = "webkit"
        record["run"] = run
        record["loadavg"] = loadAverage()
        record["wall_s"] = (now() - started) / 1000
        append(json(record))
        next()
    }

    func sendFrame() {
        sent.append(now())
        web.evaluateJavaScript("TesselRuntime.receive(__planJson(\(frameIndex)))", completionHandler: nil)
        frameIndex += 1
    }

    func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
        if message.name == "ipc" {
            received.append(now())
            if received.count < bridgeFrames { sendFrame(); return }
            let warm = 10
            let roundTrips = zip(sent.dropFirst(warm), received.dropFirst(warm)).map { $1 - $0 }
            finish([
                "bridge_native": true,
                "frames": roundTrips.count,
                "round_trip_ms": [
                    "mean": roundTrips.reduce(0, +) / Double(roundTrips.count),
                    "p50": percentile(roundTrips, 0.5), "p95": percentile(roundTrips, 0.95),
                    "p99": percentile(roundTrips, 0.99), "max": roundTrips.max()!,
                ],
                "cold_first_ms": received[0] - sent[0],
            ])
            return
        }
        let body = message.body as! String
        let result = try! JSONSerialization.jsonObject(with: body.data(using: .utf8)!) as! [String: Any]
        if result["bridgeReady"] as? Bool == true {
            bridgeFrames = (run["frames"] as? Int ?? 300) + 10
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { self.sendFrame() }
            return
        }
        finish(result)
    }
}

let app = NSApplication.shared
app.setActivationPolicy(.accessory)
let host = Host()
let screen = NSScreen.main!.visibleFrame
let window = NSWindow(
    contentRect: NSRect(x: screen.maxX - 330, y: screen.maxY - 190, width: 320, height: 180),
    styleMask: .borderless, backing: .buffered, defer: false)
window.level = .floating
window.ignoresMouseEvents = true
window.backgroundColor = .black
window.orderFrontRegardless()
host.window = window
DispatchQueue.main.async { host.next() }
app.run()
