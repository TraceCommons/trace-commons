import Foundation
import TCBridge
import TCShellCore

struct InsightsServiceRouter: Sendable {
    let selection: InsightsStoreSelection
    private let gate: InsightsServiceGate

    init(selection: InsightsStoreSelection,
         nativeCall: @escaping @Sendable (InsightsRequest) throws -> InsightsResponse = { try TCInsights.call($0) },
         startScope: @escaping @Sendable (URL) -> Bool = { $0.startAccessingSecurityScopedResource() },
         stopScope: @escaping @Sendable (URL) -> Void = { $0.stopAccessingSecurityScopedResource() }) {
        self.selection = selection
        gate = InsightsServiceGate(nativeCall: nativeCall, startScope: startScope, stopScope: stopScope)
    }

    func call(_ request: InsightsRequest) async throws -> InsightsResponse {
        guard selection.refusal == nil else { throw InsightsError.invalidResponse }
        let routed = InsightsRequest(storeDirectory: selection.storeDirectory, operation: request.operation)
        return try await gate.call(routed)
    }
}

private actor InsightsServiceGate {
    private let nativeCall: @Sendable (InsightsRequest) throws -> InsightsResponse
    private let startScope: @Sendable (URL) -> Bool
    private let stopScope: @Sendable (URL) -> Void

    init(nativeCall: @escaping @Sendable (InsightsRequest) throws -> InsightsResponse,
         startScope: @escaping @Sendable (URL) -> Bool = { $0.startAccessingSecurityScopedResource() },
         stopScope: @escaping @Sendable (URL) -> Void = { $0.stopAccessingSecurityScopedResource() }) {
        self.nativeCall = nativeCall
        self.startScope = startScope
        self.stopScope = stopScope
    }

    func call(_ request: InsightsRequest) throws -> InsightsResponse {
        let nativeCall = nativeCall, startScope = startScope, stopScope = stopScope
        let file = (request.operation.file ?? request.operation.repository).map(URL.init(fileURLWithPath:))
        let scoped = file.map(startScope) ?? false
        defer { if scoped, let file { stopScope(file) } }
        return try nativeCall(request)
    }
}
