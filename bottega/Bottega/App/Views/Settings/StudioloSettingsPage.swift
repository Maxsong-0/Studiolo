// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import AppKit
import SwiftUI

/// Settings › Studiolo: which vault the sidebar reads its archives from, and where the
/// web app is. Everything else about Studiolo is set in Studiolo.
struct StudioloSettingsPage: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        @Bindable var settings = model.settings
        let isDefault = settings.studioloVaultPath == AppSettings.defaultStudioloVaultPath
        ChromeSection(title: "Vault") {
            ChromeCard {
                ChromeRow(title: "Vault folder", detail: vaultDetail) {
                    HStack(spacing: 6) {
                        if !isDefault {
                            Button {
                                model.setStudioloVault(AppSettings.defaultStudioloVaultPath)
                            } label: {
                                Image(systemName: "arrow.uturn.backward.circle")
                                    .font(.system(size: 12, weight: .medium))
                                    .foregroundStyle(Chrome.secondaryText)
                                    .frame(width: 24, height: 24)
                                    .contentShape(.rect)
                            }
                            .buttonStyle(.plain)
                            .help("Back to \((AppSettings.defaultStudioloVaultPath as NSString).abbreviatingWithTildeInPath)")
                            .accessibilityLabel(Text("Use the default vault folder"))
                        }
                        Button("Change…") { chooseVault() }
                            .buttonStyle(.glass)
                    }
                }
            }
            Text("Each project in the sidebar can be linked to one of the vault's archives: its status shows on the project's icon, and the archives no project is linked to gather at the foot of the list. Only the archives' dashboards are read, and the vault is written only when you create an archive.")
                .font(.system(size: 11))
                .foregroundStyle(Chrome.secondaryText)
                .padding(.horizontal, 4)
        }
        ChromeSection(title: "Web app") {
            ChromeCard {
                ChromeRow(title: "Address", detail: "Open in Studiolo goes to the archive's board here.") {
                    TextField("", text: $settings.studioloWebURL, prompt: Text(verbatim: AppSettings.defaultStudioloWebURL))
                        .textFieldStyle(.roundedBorder)
                        .font(.system(size: 12, design: .monospaced))
                        .frame(width: 240)
                }
            }
        }
    }

    /// The vault row's second line: where it is and what came of reading it.
    private var vaultDetail: String {
        let studiolo = model.studiolo
        let shown = (model.settings.studioloVaultPath as NSString).abbreviatingWithTildeInPath
        guard let root = studiolo.root, StudioloVault.isVault(root) else {
            return "\(shown) · No \(StudioloIndex.projectsFolder) folder here, so Studiolo stays out of the sidebar."
        }
        guard studiolo.isAvailable else { return "\(shown) · Reading…" }
        let count = studiolo.archives.count
        return "\(shown) · \(count) \(count == 1 ? "archive" : "archives")"
    }

    private func chooseVault() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = false
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = false
        panel.directoryURL = model.studiolo.root
        panel.message = "Choose your Studiolo vault: the folder that holds \(StudioloIndex.projectsFolder)."
        panel.prompt = "Use as Vault"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        model.setStudioloVault(url.path)
    }
}
