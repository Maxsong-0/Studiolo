// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import BottegaKit
import BottegaLink
import SwiftUI

struct DevicesView: View {
    @Environment(WorkshopModel.self) private var model

    var body: some View {
        let devices = model.state.devices.values.sorted { ($0.at, $0.id) < ($1.at, $1.id) }
        if devices.isEmpty {
            ContentUnavailableView("No Devices", systemImage: Page.devices.symbol, description: Text("Devices registered with LMAS show up here."))
        } else {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 14) {
                    ForEach(devices) { DeviceBox(device: $0, isThisOne: $0.id == model.thisDevice) }
                }
                .frame(maxWidth: 720)
                .frame(maxWidth: .infinity)
                .padding(24)
            }
        }
    }
}

private struct DeviceBox: View {
    let device: Device
    let isThisOne: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Image(systemName: device.kind == "mac" ? "macbook" : "iphone").foregroundStyle(.secondary)
                Text(verbatim: UntrustedText.line(device.name, limit: 64)).font(.headline)
                if isThisOne {
                    RegisterAgainButton().controlSize(.small)
                    Text("This Mac").font(.caption).padding(.horizontal, 6).padding(.vertical, 1).background(.quaternary, in: .capsule)
                }
                Spacer()
                Text(verbatim: UntrustedText.line(device.id, limit: 24)).font(.caption.monospaced()).foregroundStyle(.secondary)
            }
            Grid(alignment: .leading, horizontalSpacing: 14, verticalSpacing: 5) {
                fact("Kind", UntrustedText.line(device.kind, limit: 24))
                fact("Registered", Wording.exactTime(device.at))
                fact("Last used", device.lastIntentAt.map(Wording.exactTime) ?? "Not yet")
                fact("Intents", device.refusals == 0 ? "\(device.intents)" : "\(device.intents), \(device.refusals) refused")
                key("Daily key", device.dailyKey)
                key("Approval key", device.approvalKey)
            }
            .font(.callout)
        }
        .padding(16)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(.background.secondary, in: .rect(cornerRadius: 12))
        .overlay { RoundedRectangle(cornerRadius: 12).strokeBorder(.separator) }
    }

    private func fact(_ name: String, _ value: String) -> some View {
        GridRow {
            Text(name).foregroundStyle(.secondary)
            Text(verbatim: value).monospacedDigit().textSelection(.enabled)
        }
    }

    /// Enough to compare by eye; all of it on hover.
    private func key(_ name: String, _ hex: String) -> some View {
        let hex = UntrustedText.line(hex, limit: 200)
        return GridRow {
            Text(name).foregroundStyle(.secondary)
            Text(verbatim: "\(hex.prefix(10))…\(hex.suffix(10))").monospaced().help(Text(verbatim: hex))
        }
    }
}

struct RegisterAgainButton: View {
    @Environment(WorkshopModel.self) private var model
    @State private var isAsking = false

    var body: some View {
        Button("Register Again…") { isAsking = true }
            .confirmationDialog("Register this Mac again with new keys?", isPresented: $isAsking) {
                Button("Register Again") { model.forgetDevice() }
            } message: {
                Text("Needed when the fingers enrolled in Touch ID have changed, or LMAS no longer knows this Mac. LMAS asks you to confirm the new keys.")
            }
    }
}

/// This Mac is not one of LMAS's devices yet.
struct RegistrationView: View {
    @Environment(WorkshopModel.self) private var model
    @State private var name = Host.current().localizedName ?? "Mac"

    var body: some View {
        VStack(spacing: 18) {
            Image(systemName: "touchid").font(.system(size: 44)).foregroundStyle(.tint)
            Text("Register This Mac").font(.title2.weight(.semibold))
            Text("Bottega makes two keys in this Mac's Secure Enclave: one for what you say, and one for what you approve, which needs your finger every time. LMAS asks you to confirm in its own window.")
                .multilineTextAlignment(.center)
                .foregroundStyle(.secondary)
                .frame(maxWidth: 420)
            TextField("Name", text: $name)
                .textFieldStyle(.roundedBorder)
                .frame(width: 280)
                .onSubmit(register)
            if let failure = model.registrationFailure {
                Text(verbatim: UntrustedText.line(failure.text, limit: 400))
                    .font(.callout)
                    .foregroundStyle(.orange)
                    .frame(maxWidth: 420)
            }
            if model.status == .registering {
                ProgressView().controlSize(.small)
            } else {
                Button("Register with Touch ID", action: register)
                    .buttonStyle(.glassProminent)
                    .disabled(Registration.usableName(name) == nil)
            }
        }
        .padding(40)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func register() {
        if let name = Registration.usableName(name) { model.register(name: name) }
    }
}
