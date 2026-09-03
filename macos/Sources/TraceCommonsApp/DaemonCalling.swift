import Foundation
import TCBridge

/// The whole of what `DaemonClient` needs from the daemon: issue one call,
/// get one JSON frame back, and open a preview.
///
/// `TCDaemon` is the only implementation that ships. This exists so the
/// *sending* half of a call -- the method name and the parameter bytes --
/// has somewhere to be asserted. Those are the two things the daemon reads,
/// they are refused as a whole when either is wrong (`set_settings` fails
/// the entire object on one unrecognised key), and until this protocol
/// existed the only way to observe them was to start a real daemon against
/// a real state directory.
///
/// Deliberately not a wrapper with its own behaviour: it declares exactly
/// `TCDaemon`'s existing signatures, so the conformance below is empty and
/// there is nothing here that can drift from what the bridge actually does.
protocol DaemonCalling: AnyObject {
    func call(_ method: String, params paramsJSON: String) -> String
    func openPreview(entryID: String) throws -> TCPreview
}

extension TCDaemon: DaemonCalling {}
