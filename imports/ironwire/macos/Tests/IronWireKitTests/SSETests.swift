//! Reading the event stream without tripping over its framing.
//
// `control.rs` sends `: connected` on attach and `: lagged N` when the bus
// dropped events on a slow subscriber. A client that treats those as data tries
// to parse `connected` as JSON every time somebody connects.

import XCTest

@testable import IronWireKit

final class SSETests: XCTestCase {
    func test_the_connected_frame_is_framing_and_not_an_event() {
        var decoder = SSEDecoder()
        XCTAssertEqual(decoder.feed(": connected"), .comment("connected"))
        XCTAssertEqual(decoder.feed(""), .pending, "a comment must not dispatch an empty event")
    }

    /// The bus is lossy on purpose: a subscriber 256 events behind is not
    /// showing anyone anything useful. This frame says so, and is not data.
    func test_the_lag_frame_is_framing_too() {
        var decoder = SSEDecoder()
        XCTAssertEqual(decoder.feed(": lagged 12"), .comment("lagged 12"))
    }

    func test_a_data_line_is_delivered_when_the_blank_line_arrives() {
        var decoder = SSEDecoder()
        XCTAssertEqual(decoder.feed(#"data: {"type":"health"}"#), .pending)
        XCTAssertEqual(decoder.feed(""), .event(#"{"type":"health"}"#))
    }

    func test_a_second_event_is_not_contaminated_by_the_first() {
        var decoder = SSEDecoder()
        _ = decoder.feed("data: one")
        XCTAssertEqual(decoder.feed(""), .event("one"))
        _ = decoder.feed("data: two")
        XCTAssertEqual(decoder.feed(""), .event("two"))
    }

    /// SSE allows a payload to span several `data:` lines. This stream does not
    /// send them, but a decoder that silently dropped all but the last would
    /// corrupt the payload rather than fail visibly if it ever did.
    func test_a_payload_split_across_data_lines_is_rejoined() {
        var decoder = SSEDecoder()
        _ = decoder.feed("data: {")
        _ = decoder.feed(#"data: "type":"health""#)
        _ = decoder.feed("data: }")
        XCTAssertEqual(decoder.feed(""), .event("{\n\"type\":\"health\"\n}"))
    }

    func test_exactly_one_leading_space_belongs_to_the_framing() {
        var decoder = SSEDecoder()
        _ = decoder.feed("data:  padded")
        XCTAssertEqual(decoder.feed(""), .event(" padded"), "only the first space is framing")
    }

    /// Fields this stream never sends. Ignoring them beats guessing.
    func test_other_sse_fields_are_ignored_without_dispatching_anything() {
        var decoder = SSEDecoder()
        XCTAssertEqual(decoder.feed("event: routed"), .pending)
        XCTAssertEqual(decoder.feed("id: 7"), .pending)
        XCTAssertEqual(decoder.feed("retry: 1000"), .pending)
        XCTAssertEqual(decoder.feed(""), .pending)
    }

    func test_repeated_blank_lines_do_not_produce_empty_events() {
        var decoder = SSEDecoder()
        XCTAssertEqual(decoder.feed(""), .pending)
        XCTAssertEqual(decoder.feed(""), .pending)
    }

    /// The property that matters most: a payload the app cannot read costs that
    /// one event and nothing else. The stream stays up, and the next poll is
    /// still the source of truth.
    func test_a_payload_that_is_not_json_costs_one_event_and_not_the_stream() {
        var decoder = SSEDecoder()
        _ = decoder.feed("data: {not json")
        guard case .event(let payload) = decoder.feed("") else {
            return XCTFail("the decoder should still frame it")
        }
        XCTAssertNil(try? controlDecoder().decode(Event.self, from: Data(payload.utf8)))

        _ = decoder.feed(#"data: {"type":"failed","at":"2026-08-08T12:00:00Z","detail":"x"}"#)
        guard case .event(let next) = decoder.feed("") else {
            return XCTFail("the next event should still arrive")
        }
        XCTAssertNotNil(try? controlDecoder().decode(Event.self, from: Data(next.utf8)))
    }
}
