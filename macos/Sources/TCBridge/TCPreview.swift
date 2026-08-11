import CTraceCommons
import Foundation

/// One open preview: the redacted transcript body, its summary, and a local
/// search over the body.
///
/// Every `const char*` an accessor returns is BORROWED and valid only until
/// `tc_preview_free`, so each one is copied into a Swift `String` before this
/// type hands anything back. Nothing outside this file holds a pointer into
/// the preview allocation.
///
/// Ownership discipline, per the header: the pointer-validity check the ABI
/// runs is keyed on the pointer VALUE and cannot make a concurrent free safe.
/// So this type is used from one place at a time -- opened on a background
/// task, handed to the main actor, closed there -- and `close()` is
/// idempotent.
public final class TCPreview {
    private var pointer: OpaquePointer?

    internal init(pointer: OpaquePointer) {
        self.pointer = pointer
    }

    /// The redacted transcript. This is the C ABI's one deliberate content
    /// exemption; it must never reach a log line, a notification, or a
    /// history record.
    public var body: String {
        guard let pointer, let c = tc_preview_body(pointer) else { return "" }
        return String(cString: c)
    }

    /// Counts, sizes and the opening prompt, as JSON.
    public var summaryJSON: String {
        guard let pointer, let c = tc_preview_summary_json(pointer) else { return "{}" }
        return String(cString: c)
    }

    /// Searches the redacted body for `needle`, returning UTF-8 BYTE offsets
    /// of non-overlapping, left-to-right matches. An empty needle matches
    /// nothing. Returns nil on error (the ABI reports -1).
    public func search(_ needle: String) -> [Int]? {
        guard let pointer else { return nil }
        var matchesPtr: UnsafeMutablePointer<CChar>?
        let count: Int32 = needle.withCString { cNeedle in
            withUnsafeMutablePointer(to: &matchesPtr) { out in
                tc_preview_search(pointer, cNeedle, out)
            }
        }
        // On error the ABI sets *matches_json to NULL: there is nothing to
        // free, and nothing to parse.
        guard count >= 0 else { return nil }
        guard let matchesPtr else { return [] }
        defer { tc_string_free(matchesPtr) }
        let json = String(cString: matchesPtr)
        guard let data = json.data(using: .utf8),
              let offsets = try? JSONDecoder().decode([Int].self, from: data)
        else { return nil }
        return offsets
    }

    /// Frees the preview. Invalidates every string the ABI previously
    /// returned for it -- which is why this type copies each one out
    /// immediately. Safe to call more than once.
    public func close() {
        guard let p = pointer else { return }
        pointer = nil
        tc_preview_free(p)
    }

    deinit {
        close()
    }
}
