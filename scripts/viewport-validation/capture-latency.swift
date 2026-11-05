// Item 7. Reads a barcode out of the window as ScreenCaptureKit delivers it. This is the compositor's output as
// seen by a capture API: it is NOT light leaving the panel. swiftc -O capture-latency.swift -o capture-latency
//   capture-latency locate <pid>              -> window id and bounds (points)
//   capture-latency run <pid> <x0> <y0> <cell> <cells> <seconds> <out.jsonl>   coordinates in window points, top-left origin
import AppKit; import ScreenCaptureKit; import CoreMedia
let args = CommandLine.arguments
func windows(_ pid: Int32) -> [[String: Any]] { ((CGWindowListCopyWindowInfo([.optionAll], kCGNullWindowID) as? [[String: Any]]) ?? []).filter { ($0[kCGWindowOwnerPID as String] as? Int32) == pid && (($0[kCGWindowLayer as String] as? Int) ?? 1) == 0 && (($0[kCGWindowBounds as String] as? [String: Double])?["Width"] ?? 0) > 100 } }
if args[1] == "locate" { let w = windows(Int32(args[2])!); let d = try! JSONSerialization.data(withJSONObject: w.map { ["id": $0[kCGWindowNumber as String]!, "bounds": $0[kCGWindowBounds as String]!, "onscreen": $0[kCGWindowIsOnscreen as String] ?? false] }); print(String(data: d, encoding: .utf8)!); exit(0) }
final class Out: NSObject, SCStreamOutput, SCStreamDelegate {
  let x0, y0, cell: Double; let cells: Int; let scale: Double; let path: String; var rows: [String] = []; var last = -1; var seen = 0
  init(x0: Double, y0: Double, cell: Double, cells: Int, scale: Double, path: String) { self.x0 = x0; self.y0 = y0; self.cell = cell; self.cells = cells; self.scale = scale; self.path = path }
  func stream(_ s: SCStream, didOutputSampleBuffer b: CMSampleBuffer, of type: SCStreamOutputType) {
    guard type == .screen, let att = CMSampleBufferGetSampleAttachmentsArray(b, createIfNecessary: false) as? [[SCStreamFrameInfo: Any]], let a = att.first, let raw = a[.status] as? Int, SCFrameStatus(rawValue: raw) == .complete, let img = CMSampleBufferGetImageBuffer(b) else { return }
    let display = (a[.displayTime] as? UInt64) ?? 0; let arrival = mach_absolute_time()
    CVPixelBufferLockBaseAddress(img, .readOnly); defer { CVPixelBufferUnlockBaseAddress(img, .readOnly) }
    let base = CVPixelBufferGetBaseAddress(img)!.assumingMemoryBound(to: UInt8.self); let bpr = CVPixelBufferGetBytesPerRow(img); let w = CVPixelBufferGetWidth(img), h = CVPixelBufferGetHeight(img)
    var code = 0
    for k in 0..<cells { let x = Int((x0 + (Double(k) + 0.5) * cell) * scale), y = Int((y0 + 0.5 * cell) * scale); if x >= w || y >= h { continue }; let p = base + y * bpr + x * 4; if (Int(p[0]) + Int(p[1]) + Int(p[2])) / 3 > 128 { code |= 1 << k } }
    seen += 1
    if code != last { rows.append("[\(display),\(arrival),\(code)]"); last = code }
  }
  func flush(info: String) { try! (["{\"info\":\(info),\"frames_seen\":\(seen),\"rows\":[" + rows.joined(separator: ",") + "]}"].joined()).write(toFile: path, atomically: true, encoding: .utf8) }
}
let pid = Int32(args[2])!, X0 = Double(args[3])!, Y0 = Double(args[4])!, CELL = Double(args[5])!, CELLS = Int(args[6])!, SECS = Double(args[7])!, PATH = args[8]
Task {
  let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: false)
  let ids = windows(pid).map { $0[kCGWindowNumber as String] as! UInt32 }
  guard let win = content.windows.first(where: { ids.contains($0.windowID) }) else { print("no window"); exit(2) }
  let f = SCContentFilter(desktopIndependentWindow: win); let cfg = SCStreamConfiguration()
  let scale = Double(f.pointPixelScale); cfg.width = Int(win.frame.width * scale); cfg.height = Int(win.frame.height * scale); cfg.minimumFrameInterval = CMTime(value: 1, timescale: 120); cfg.queueDepth = 5; cfg.showsCursor = false; cfg.pixelFormat = kCVPixelFormatType_32BGRA
  let o = Out(x0: X0, y0: Y0, cell: CELL, cells: CELLS, scale: scale, path: PATH)
  let s = SCStream(filter: f, configuration: cfg, delegate: o); try s.addStreamOutput(o, type: .screen, sampleHandlerQueue: DispatchQueue(label: "cap"))
  var tb = mach_timebase_info_data_t(); mach_timebase_info(&tb)
  try await s.startCapture()
  // Maps mach ticks to the epoch clock the spans use.
  let e = Date().timeIntervalSince1970 * 1000; let m = mach_absolute_time()
  print("started"); fflush(stdout)
  try await Task.sleep(nanoseconds: UInt64(SECS * 1e9))
  try await s.stopCapture()
  o.flush(info: "{\"scale\":\(scale),\"epoch_ms\":\(e),\"mach\":\(m),\"numer\":\(tb.numer),\"denom\":\(tb.denom),\"w\":\(cfg.width),\"h\":\(cfg.height)}")
  exit(0)
}
RunLoop.main.run()
