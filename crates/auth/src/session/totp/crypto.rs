//! HMAC-SHA1 TOTP 纯匹配；不读取时钟、不消费时间步、不写入任何存储。

use super::secrets::{TotpCode, TotpSeed};
use hmac::{Hmac, Mac};
use sha1::Sha1;
use subtle::{Choice, ConditionallySelectable, ConstantTimeEq};
use zeroize::{Zeroize, Zeroizing};

pub(in crate::session) fn matching_step(
    seed: &TotpSeed,
    code: &TotpCode,
    now_ms: i64,
    last_step: i64,
) -> Option<i64> {
    if now_ms < 0 {
        return None;
    }
    let current = now_ms / 30_000;
    let mut candidates = [(0, Choice::from(0)); 3];
    for (slot, delta) in [-1_i64, 0, 1].into_iter().enumerate() {
        let step = current.checked_add(delta).filter(|step| *step >= 0);
        // 第零步也执行三次计算与比较；无效负步只用掩码排除，不能下溢。
        let counter = step.unwrap_or(0);
        let mut number = Zeroizing::new(truncated(seed, counter as u64)? % 1_000_000);
        let mut expected = Zeroizing::new([b'0'; 6]);
        for byte in expected.iter_mut().rev() {
            *byte += (*number % 10) as u8;
            *number /= 10;
        }
        let matched =
            expected.as_slice().ct_eq(code.bytes()) & Choice::from(u8::from(step.is_some()));
        candidates[slot] = (counter, matched);
    }
    select_step(candidates, last_step)
}

fn select_step(candidates: [(i64, Choice); 3], last_step: i64) -> Option<i64> {
    let mut selected = -1_i64;
    let mut found = Choice::from(0);
    let mut consumed = Choice::from(0);
    for (step, matched) in candidates {
        let newer = Choice::from(u8::from(step > last_step));
        consumed |= matched & !newer;
        found |= matched & newer;
        // 候选按时间步递增，匹配时覆盖为最大值；不因首个命中提前返回。
        selected = i64::conditional_select(&selected, &step, matched & newer);
    }
    if bool::from(found & !consumed) {
        Some(selected)
    } else {
        None
    }
}

fn truncated(seed: &TotpSeed, step: u64) -> Option<u32> {
    // SHA1 仅用于标准 HMAC，不是密码哈希；固定 20 字节密钥满足该 Adapter。
    let mut mac = Hmac::<Sha1>::new_from_slice(seed.bytes()).ok()?;
    mac.update(&step.to_be_bytes());
    let mut result = mac.finalize().into_bytes();
    let offset = usize::from(result[19] & 15);
    let mut selected = Zeroizing::new([0_u8; 4]);
    selected.copy_from_slice(&result[offset..offset + 4]);
    result[..].zeroize();
    Some(u32::from_be_bytes(*selected) & 0x7fff_ffff)
}

#[cfg(test)]
#[path = "crypto_tests.rs"]
mod tests;
