// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Darwin
import Dispatch
import Foundation
import Synchronization

/// Connects to `ui.sock`.
public struct UnixSocketConnector: Connector {
    private let path: String

    public init(path: String) { self.path = path }

    public func connect() async throws -> any LineChannel {
        // Connecting to a local socket does not wait for the other side, so it is done right here.
        let descriptor = socket(AF_UNIX, SOCK_STREAM, 0)
        guard descriptor >= 0 else { throw POSIXError.now }
        do {
            var address = sockaddr_un()
            address.sun_family = sa_family_t(AF_UNIX)
            let bytes = Array(path.utf8)
            guard !bytes.isEmpty, bytes.count < MemoryLayout.size(ofValue: address.sun_path), !bytes.contains(0) else {
                throw POSIXError(.ENAMETOOLONG)
            }
            withUnsafeMutableBytes(of: &address.sun_path) { $0.copyBytes(from: bytes) }
            let connected = withUnsafePointer(to: &address) { pointer in
                pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                    Darwin.connect(descriptor, $0, socklen_t(MemoryLayout<sockaddr_un>.size))
                }
            }
            guard connected == 0 else { throw POSIXError.now }
            // A write after LMAS hung up is an error to handle, not a signal.
            var yes: Int32 = 1
            guard setsockopt(descriptor, SOL_SOCKET, SO_NOSIGPIPE, &yes, socklen_t(MemoryLayout<Int32>.size)) == 0 else {
                throw POSIXError.now
            }
            _ = fcntl(descriptor, F_SETFD, FD_CLOEXEC)
        } catch {
            Darwin.close(descriptor)
            throw error
        }
        return UnixSocketChannel(descriptor: descriptor)
    }
}

extension POSIXError {
    fileprivate static var now: POSIXError { POSIXError(.init(rawValue: errno) ?? .EIO) }
}

/// DispatchIO does the reading and writing on a queue of its own and closes the descriptor when both are over.
final class UnixSocketChannel: LineChannel {
    private let queue = DispatchQueue(label: "me.studiolo.bottega.ui-sock")
    private let channel: DispatchIO
    private let splitter = Mutex(LineSplitter())

    init(descriptor: Int32) {
        channel = DispatchIO(type: .stream, fileDescriptor: descriptor, queue: queue) { _ in Darwin.close(descriptor) }
        channel.setLimit(lowWater: 1)
    }

    func lines() -> AsyncThrowingStream<Data, any Error> {
        AsyncThrowingStream { continuation in
            continuation.onTermination = { [channel] _ in channel.close(flags: .stop) }
            channel.read(offset: 0, length: .max, queue: queue) { [self] done, data, error in
                if let data, !data.isEmpty {
                    do {
                        for line in try splitter.withLock({ try $0.take(Data(data)) }) { continuation.yield(line) }
                    } catch {
                        return continuation.finish(throwing: error)
                    }
                }
                guard done else { return }
                // ECANCELED is this side closing.
                if error == 0 || error == ECANCELED {
                    continuation.finish()
                } else {
                    continuation.finish(throwing: POSIXError(.init(rawValue: error) ?? .EIO))
                }
            }
        }
    }

    func send(_ line: Data) async throws {
        let bytes = line.withUnsafeBytes { DispatchData(bytes: $0) }
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<Void, any Error>) in
            channel.write(offset: 0, data: bytes, queue: queue) { done, _, error in
                guard done else { return }
                if error == 0 {
                    continuation.resume()
                } else {
                    continuation.resume(throwing: POSIXError(.init(rawValue: error) ?? .EIO))
                }
            }
        }
    }

    func close() { channel.close(flags: .stop) }
}
