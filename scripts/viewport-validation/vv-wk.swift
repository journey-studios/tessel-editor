// Real WKWebView bench host: tessel:// scheme served synchronously on the main thread (like wry's
// handler), with Range, in a small non-activating window. swiftc -O vv-wk.swift -o vv-wk
// ./vv-wk <runs.json> <out.jsonl> <page-dir> <rt-dir> <assets-dir> <png-dir>
import AppKit; import WebKit; import Darwin
let a = CommandLine.arguments
var runs = try! JSONSerialization.jsonObject(with: Data(contentsOf: URL(fileURLWithPath: a[1]))) as! [[String: Any]]
let outURL = URL(fileURLWithPath: a[2]); let pageDir = a[3], rtDir = a[4], assetsDir = a[5], pngDir = a[6]
func append(_ s: String) { if !FileManager.default.fileExists(atPath: outURL.path) { FileManager.default.createFile(atPath: outURL.path, contents: nil) }; let h = try! FileHandle(forWritingTo: outURL); h.seekToEndOfFile(); h.write((s + "\n").data(using: .utf8)!); h.closeFile() }
func mime(_ p: String) -> String { let e = (p as NSString).pathExtension.lowercased(); return ["html": "text/html", "js": "text/javascript", "png": "image/png", "jpg": "image/jpeg", "webp": "image/webp", "json": "application/json", "mp4": "video/mp4", "webm": "video/webm", "mov": "video/quicktime"][e] ?? "application/octet-stream" }
var serveMs: [Double] = []
class Scheme: NSObject, WKURLSchemeHandler {
  func webView(_ w: WKWebView, start t: WKURLSchemeTask) {
    let t0 = DispatchTime.now().uptimeNanoseconds
    let u = URLComponents(url: t.request.url!, resolvingAgainstBaseURL: false)!
    var path = ""
    if u.path.hasPrefix("/page/") { path = pageDir + "/" + u.path.dropFirst(6) }
    else if u.path.hasPrefix("/rt/") { let c = u.path.split(separator: "/"); path = rtDir + "/" + c[1] + "/dist/runtime.js" }
    else if u.path == "/asset" { path = u.queryItems!.first { $0.name == "p" }!.value! }
    else if u.path == "/index.html" { path = pageDir + "/index.html" }
    guard let fh = FileHandle(forReadingAtPath: path) else { t.didFailWithError(NSError(domain: "x", code: 404)); return }
    defer { fh.closeFile() }
    let size = Int(fh.seekToEndOfFile()); var start = 0, end = size - 1, status = 200
    var headers = ["Content-Type": mime(path), "Cache-Control": "no-store", "Accept-Ranges": "bytes"]
    if let r = t.request.value(forHTTPHeaderField: "Range"), r.hasPrefix("bytes=") {
      let p = r.dropFirst(6).split(separator: "-", omittingEmptySubsequences: false)
      start = Int(p[0]) ?? 0; if p.count > 1, let e = Int(p[1]) { end = min(e, size - 1) }
      status = 206; headers["Content-Range"] = "bytes \(start)-\(end)/\(size)"
    }
    fh.seek(toFileOffset: UInt64(start)); let d = fh.readData(ofLength: end - start + 1)
    headers["Content-Length"] = "\(d.count)"
    t.didReceive(HTTPURLResponse(url: t.request.url!, statusCode: status, httpVersion: "HTTP/1.1", headerFields: headers)!); t.didReceive(d); t.didFinish()
    if u.path == "/asset" { serveMs.append(Double(DispatchTime.now().uptimeNanoseconds - t0) / 1e6) }
  }
  func webView(_ w: WKWebView, stop t: WKURLSchemeTask) {}
}
func footprint(_ pid: Int32) -> Double? { guard pid > 0 else { return nil }; var info = rusage_info_v4(); let r = withUnsafeMutablePointer(to: &info) { $0.withMemoryRebound(to: rusage_info_t?.self, capacity: 1) { proc_pid_rusage(pid, RUSAGE_INFO_V4, $0) } }; return r == 0 ? Double(info.ri_phys_footprint) / 1048576 : nil }
class Host: NSObject, WKScriptMessageHandler, WKNavigationDelegate {
  var web: WKWebView!; var window: NSWindow!; var timer: Timer?; var run: [String: Any] = [:]
  func pid(_ key: String) -> Int32 { if web.responds(to: Selector(key)) { return (web.value(forKey: key) as? NSNumber)?.int32Value ?? 0 }; return 0 }
  func cfgJson() -> String { String(data: try! JSONSerialization.data(withJSONObject: run), encoding: .utf8)! }
  func next() {
    if runs.isEmpty { NSApp.terminate(nil); return }
    run = runs.removeFirst(); run["assetsDir"] = assetsDir; serveMs = []
    let b64 = (try! JSONSerialization.data(withJSONObject: run)).base64EncodedString().addingPercentEncoding(withAllowedCharacters: .alphanumerics)!
    let cfg = WKWebViewConfiguration(); cfg.setURLSchemeHandler(Scheme(), forURLScheme: "tessel")
    cfg.userContentController.add(self, name: "done"); cfg.userContentController.add(self, name: "ctl")
    cfg.userContentController.addUserScript(WKUserScript(source: "window.__vvFinish=(r)=>webkit.messageHandlers.done.postMessage(JSON.stringify(r));window.__vvCtl=(s)=>webkit.messageHandlers.ctl.postMessage(s);", injectionTime: .atDocumentStart, forMainFrameOnly: true))
    web?.navigationDelegate = nil; web?.removeFromSuperview(); web?.configuration.userContentController.removeAllScriptMessageHandlers()
    web = WKWebView(frame: window.contentView!.bounds, configuration: cfg); web.autoresizingMask = [.width, .height]; web.navigationDelegate = self
    window.contentView!.addSubview(web)
    timer?.invalidate(); timer = Timer.scheduledTimer(withTimeInterval: (run["timeoutSec"] as? Double) ?? 240, repeats: false) { _ in append("{\"engine\":\"webkit\",\"fatal\":\"timeout\",\"cfg\":\(self.cfgJson())}"); self.next() }
    web.load(URLRequest(url: URL(string: "tessel://localhost/index.html?cfg=\(b64)")!))
  }
  func webViewWebContentProcessDidTerminate(_ w: WKWebView) { append("{\"engine\":\"webkit\",\"fatal\":\"webcontent_terminated\",\"cfg\":\(cfgJson())}"); next() }
  func userContentController(_ c: WKUserContentController, didReceive m: WKScriptMessage) {
    let body = m.body as! String
    if m.name == "ctl" {
      let o = try! JSONSerialization.jsonObject(with: body.data(using: .utf8)!) as! [String: Any]
      if o["op"] as? String == "mem" {
        let web_mb = footprint(pid("_webProcessIdentifier")); let gpu = footprint(pid("_gpuProcessIdentifier"))
        web.evaluateJavaScript("window.__vvMemReply({web_mb:\(web_mb.map { String($0) } ?? "null"),gpu_mb:\(gpu.map { String($0) } ?? "null")})", completionHandler: nil)
      } else if o["op"] as? String == "save" { try? Data(base64Encoded: o["b64"] as! String)!.write(to: URL(fileURLWithPath: pngDir + "/wk-" + (o["name"] as! String))) }
      return
    }
    let sv = serveMs.sorted(); let med = sv.isEmpty ? 0 : sv[sv.count / 2]; let mx = sv.last ?? 0
    append(String(body.dropLast()) + ",\"engine\":\"webkit\",\"native_serve\":{\"n\":\(sv.count),\"p50\":\(med),\"max\":\(mx)}}"); DispatchQueue.main.async { self.next() }
  }
}
let app = NSApplication.shared; app.setActivationPolicy(.accessory)
let host = Host(); let screen = NSScreen.main!.visibleFrame
let win = NSWindow(contentRect: NSRect(x: screen.maxX - 330, y: screen.maxY - 190, width: 320, height: 180), styleMask: .borderless, backing: .buffered, defer: false)
win.level = .floating; win.ignoresMouseEvents = true; win.backgroundColor = .black; win.orderFrontRegardless(); host.window = win
DispatchQueue.main.async { host.next() }
app.run()
