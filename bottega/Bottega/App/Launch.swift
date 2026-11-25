// Bottega, modified from Droppy Code. Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app. GNU AGPL v3 with additional terms, see LICENSE.
import Foundation

/// Bottega opens onto LMAS. The app it grew out of, which runs agent CLIs of its own, is still in here and opens
/// with `-classic` until its layers are taken out (vault Bottega-6, section 4); the two share no state, and neither
/// starts anything of the other's.
@main
enum Launch {
    static func main() {
        if CommandLine.arguments.contains("-classic") {
            BottegaApp.main()
        } else {
            WorkshopApp.main()
        }
    }
}
