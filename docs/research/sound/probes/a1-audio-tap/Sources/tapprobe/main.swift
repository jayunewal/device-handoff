// SPDX-License-Identifier: Apache-2.0

// A1 audio-tap probe: taps the audio of our own tone generator with a Core Audio process tap,
// checks that it is captured, and switches the tap's mute behaviour between phases.
// Touches no other app's audio and changes no system setting.
import CoreAudio
import AudioToolbox
import Foundation

if Bundle.main.bundlePath.hasSuffix(".app") {
    let log = (Bundle.main.bundlePath as NSString).deletingLastPathComponent + "/tap-run.log"
    freopen(log, "w", stdout); freopen(log, "a", stderr)
}
setvbuf(stdout, nil, _IOLBF, 0)
var tb = mach_timebase_info(); mach_timebase_info(&tb)
func ms(_ dt: Int64) -> Double { Double(dt) * Double(tb.numer) / Double(tb.denom) / 1e6 }
func stamp() -> String { let f = DateFormatter(); f.dateFormat = "HH:mm:ss"; return f.string(from: Date()) }

// MARK: tone generator child
let exe = URL(fileURLWithPath: Bundle.main.executablePath ?? CommandLine.arguments[0]).deletingLastPathComponent().appendingPathComponent("tonegen")
let child = Process(); child.executableURL = exe; child.arguments = ["32"]
let pipe = Pipe(); child.standardOutput = pipe
struct Click { let sample: Int64; let renderNow: UInt64; let outHost: UInt64 }
let lock = NSLock()
var clicks: [Click] = []
var started = false, childPid: pid_t = 0
var lineBuf = ""
pipe.fileHandleForReading.readabilityHandler = { h in
    guard let s = String(data: h.availableData, encoding: .utf8) else { return }
    lock.lock(); defer { lock.unlock() }
    lineBuf += s
    while let r = lineBuf.range(of: "\n") {
        let line = String(lineBuf[..<r.lowerBound]); lineBuf.removeSubrange(..<r.upperBound)
        let p = line.split(separator: " ")
        if p.first == "started" { started = true; print("tonegen: \(line)") }
        else if p.first == "click", p.count >= 4 { clicks.append(Click(sample: Int64(p[1])!, renderNow: UInt64(p[2])!, outHost: UInt64(p[3])!)) }
        else if !line.isEmpty { print("tonegen: \(line)") }
    }
}
func say(_ text: String, wait: Bool = true) {
    print("[\(stamp())] voice: \(text)")
    let p = Process(); p.executableURL = URL(fileURLWithPath: "/usr/bin/say"); p.arguments = [text]
    try? p.run(); if wait { p.waitUntilExit() }
}
say("Baton sound test. First you will hear a tone with clicks.")
try! child.run()
childPid = child.processIdentifier
while true { lock.lock(); let s = started; lock.unlock(); if s { break }; usleep(10_000) }

func processObject(_ pid: pid_t) -> AudioObjectID {
    var addr = AudioObjectPropertyAddress(mSelector: kAudioHardwarePropertyTranslatePIDToProcessObject, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
    var p = pid, obj = AudioObjectID(kAudioObjectUnknown), size = UInt32(MemoryLayout<AudioObjectID>.size)
    let st = AudioObjectGetPropertyData(AudioObjectID(kAudioObjectSystemObject), &addr, UInt32(MemoryLayout<pid_t>.size), &p, &size, &obj)
    if st != 0 { print("translate pid status \(st)") }
    return obj
}

// MARK: one tap phase
final class PhaseStats {
    var frames: Int64 = 0, sumSq = 0.0, peak: Float = 0, callbacks = 0, gaps = 0
    var lastSampleTime: Double = -1
    var onsets: [(UInt64, Int)] = []      // arrival host time of the IOProc call, frame offset
    var below = 0
    var firstCallback: UInt64 = 0
}

func runTap(name: String, mute: CATapMuteBehavior, readSeconds: Double, idleAfterStopSeconds: Double = 0, onStart: (() -> Void)? = nil) {
    let procObj = processObject(childPid)
    let desc = CATapDescription(stereoMixdownOfProcesses: [procObj])
    desc.name = "Baton probe tap"
    desc.isPrivate = true
    desc.muteBehavior = mute
    var tap = AudioObjectID(kAudioObjectUnknown)
    let ts = AudioHardwareCreateProcessTap(desc, &tap)
    print("\n[\(stamp())] phase \(name): process object \(procObj), create tap status \(ts), tap id \(tap)")
    guard ts == 0 else { return }
    var addr = AudioObjectPropertyAddress(mSelector: kAudioTapPropertyFormat, mScope: kAudioObjectPropertyScopeGlobal, mElement: kAudioObjectPropertyElementMain)
    var asbd = AudioStreamBasicDescription(), sz = UInt32(MemoryLayout<AudioStreamBasicDescription>.size)
    AudioObjectGetPropertyData(tap, &addr, 0, nil, &sz, &asbd)
    print("  tap format: \(asbd.mSampleRate) Hz, \(asbd.mChannelsPerFrame) ch, \(asbd.mBitsPerChannel) bit, flags 0x\(String(asbd.mFormatFlags, radix: 16))")
    let agg: [String: Any] = [kAudioAggregateDeviceNameKey: "Baton probe", kAudioAggregateDeviceUIDKey: UUID().uuidString,
                              kAudioAggregateDeviceIsPrivateKey: true, kAudioAggregateDeviceTapAutoStartKey: true,
                              kAudioAggregateDeviceTapListKey: [[kAudioSubTapUIDKey: desc.uuid.uuidString, kAudioSubTapDriftCompensationKey: true]]]
    var dev = AudioObjectID(kAudioObjectUnknown)
    let as_ = AudioHardwareCreateAggregateDevice(agg as CFDictionary, &dev)
    print("  aggregate device status \(as_), id \(dev)")
    let stats = PhaseStats()
    let chans = Int(asbd.mChannelsPerFrame)
    var procID: AudioDeviceIOProcID?
    let q = DispatchQueue(label: "io")
    let cs = AudioDeviceCreateIOProcIDWithBlock(&procID, dev, q) { _, inData, inTime, _, _ in
        let arrival = mach_absolute_time()
        let abl = UnsafeMutableAudioBufferListPointer(UnsafeMutablePointer(mutating: inData))
        if stats.firstCallback == 0 { stats.firstCallback = arrival }
        stats.callbacks += 1
        guard let b = abl.first, let d = b.mData else { return }
        let p = d.assumingMemoryBound(to: Float.self)
        let n = Int(b.mDataByteSize) / 4 / max(1, chans)
        let st = inTime.pointee.mSampleTime
        if stats.lastSampleTime >= 0 && abs(st - stats.lastSampleTime) > 1 { stats.gaps += 1 }
        stats.lastSampleTime = st + Double(n)
        for i in 0..<n {
            let v = p[i * chans]
            stats.sumSq += Double(v * v); stats.peak = max(stats.peak, abs(v))
            if abs(v) > 0.07 { if stats.below > Int(asbd.mSampleRate * 0.1) { stats.onsets.append((arrival, i)) }; stats.below = 0 } else { stats.below += 1 }
        }
        stats.frames += Int64(n)
    }
    print("  create IOProc status \(cs)")
    let ss = AudioDeviceStart(dev, procID)
    print("  [\(stamp())] reading tap (start status \(ss)) for \(readSeconds) s")
    let t0 = Date()
    onStart?()
    Thread.sleep(forTimeInterval: max(0, readSeconds - Date().timeIntervalSince(t0)))
    AudioDeviceStop(dev, procID)
    q.sync {}
    if idleAfterStopSeconds > 0 { print("  [\(stamp())] tap still exists but nobody reads it, \(idleAfterStopSeconds) s"); Thread.sleep(forTimeInterval: idleAfterStopSeconds) }
    if let procID { AudioDeviceDestroyIOProcID(dev, procID) }
    AudioHardwareDestroyAggregateDevice(dev)
    AudioHardwareDestroyProcessTap(tap)
    print("  [\(stamp())] tap destroyed")
    // results
    let rms = stats.frames > 0 ? sqrt(stats.sumSq / Double(stats.frames)) : 0
    print("  captured \(stats.frames) frames in \(stats.callbacks) callbacks (\(stats.callbacks > 0 ? stats.frames / Int64(stats.callbacks) : 0) frames each), RMS \(String(format: "%.4f", rms)) (tone alone ≈ 0.021), peak \(String(format: "%.3f", stats.peak)), sample-time gaps \(stats.gaps)")
    lock.lock(); let cl = clicks; lock.unlock()
    var lat: [Double] = [], latOut: [Double] = []
    for (arr, _) in stats.onsets {
        if let c = cl.last(where: { $0.renderNow <= arr }), ms(Int64(arr) - Int64(c.renderNow)) < 400 {
            lat.append(ms(Int64(arr) - Int64(c.renderNow))); latOut.append(ms(Int64(arr) - Int64(c.outHost)))
        }
    }
    if lat.isEmpty { print("  clicks matched: 0 (no audio captured?)") }
    else {
        let s = lat.sorted(), so = latOut.sorted()
        print("  clicks matched: \(lat.count); delay from the tone app rendering a click to the tap callback that holds it: median \(String(format: "%.1f", s[s.count / 2])) ms, min \(String(format: "%.1f", s.first!)), max \(String(format: "%.1f", s.last!)) ms")
        print("  same, against the time the tone app was told the click would reach the speaker (negative = tap has it before the speaker would play it): median \(String(format: "%.1f", so[so.count / 2])) ms, min \(String(format: "%.1f", so.first!)), max \(String(format: "%.1f", so.last!)) ms")
        print("  (delay includes up to one tap buffer of \(stats.callbacks > 0 ? String(format: "%.1f", Double(stats.frames / Int64(stats.callbacks)) / asbd.mSampleRate * 1000) : "-") ms because arrival is the callback time, not the sample's own time)")
    }
}

// Ear test with spoken cues: tone 4 s, then muted by the tap for 10 s, then tone again.
print("[\(stamp())] phase 1: tone plays, no tap (should be audible) 4 s")
Thread.sleep(forTimeInterval: 4)
runTap(name: "2 CATapMuted, 10 s (speaker should be SILENT apart from the voice)", mute: .muted, readSeconds: 10) {
    say("Muted now. The tone is still playing, but Baton is capturing it, so you should hear nothing except my voice, for ten seconds.")
}
print("\n[\(stamp())] phase 3: no tap (audible again) until the tone app ends")
say("Not muted any more. You should hear the tone again now, for about fifteen seconds.")
print("\n[\(stamp())] waiting for the tone app to end")
child.waitUntilExit()
pipe.fileHandleForReading.readabilityHandler = nil
say("Test finished.")
print("done")
