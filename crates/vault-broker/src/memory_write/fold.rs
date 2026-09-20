//! NFC + Unicode 完整 casefold + trim；再 NFC 保持键稳定。
#[path = "fold_0.rs"]
mod first;
#[path = "fold_1.rs"]
mod second;
use unicode_normalization::UnicodeNormalization as _;
pub(super) fn normalize(value: &str) -> String {
    let mut folded = String::new();
    for c in value.trim().nfc() {
        if let Some(expanded) = first::special(c).or_else(|| second::special(c)) {
            folded.push_str(expanded);
        } else {
            folded.extend(c.to_lowercase());
        }
    }
    folded.nfc().collect()
}
