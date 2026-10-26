// SPDX-License-Identifier: Apache-2.0

// Plays a quiet 440 Hz tone with a short 1 kHz click every 500 ms, for a fixed duration.
// Prints "click <sampleIndex> <renderCallHostTime> <outputHostTime>" for each click.
import AVFoundation
import Foundation

setvbuf(stdout, nil, _IOLBF, 0)
let duration = Double(CommandLine.arguments.dropFirst().first ?? "20") ?? 20
let engine = AVAudioEngine()
let sr = engine.outputNode.outputFormat(forBus: 0).sampleRate
let clickEvery = Int64(sr / 2), clickLen = Int64(sr * 0.005)
var phase = 0.0
var n: Int64 = 0
let lock = NSLock()
var events: [String] = []

let src = AVAudioSourceNode { _, ts, frames, abl -> OSStatus in
    let now = mach_absolute_time()
    let bufs = UnsafeMutableAudioBufferListPointer(abl)
    for f in 0..<Int(frames) {
        let k = (n + Int64(f)) % clickEvery
        var v = 0.03 * sin(phase)
        phase += 2 * Double.pi * 440 / sr
        if k < clickLen { v = 0.12 * sin(2 * Double.pi * 1000 * Double(k) / sr + Double.pi / 2) }
        if k == 0 {
            let out = ts.pointee.mHostTime
            lock.lock(); events.append("click \(n + Int64(f)) \(now) \(out) \(f)"); lock.unlock()
        }
        for b in bufs { b.mData!.assumingMemoryBound(to: Float.self)[f] = Float(v) }
    }
    n += Int64(frames)
    return noErr
}
engine.attach(src)
engine.connect(src, to: engine.mainMixerNode, format: AVAudioFormat(standardFormatWithSampleRate: sr, channels: 1)!)
do { try engine.start() } catch { print("error \(error)"); exit(1) }
print("started pid=\(getpid()) rate=\(sr)")
let end = Date().addingTimeInterval(duration)
while Date() < end {
    Thread.sleep(forTimeInterval: 0.05)
    lock.lock(); let e = events; events.removeAll(); lock.unlock()
    e.forEach { print($0) }
}
engine.stop()
print("stopped")
