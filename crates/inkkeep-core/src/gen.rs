//! 密碼產生。

use rand::seq::SliceRandom;
use rand::Rng;
use serde::{Deserialize, Serialize};

pub const MIN_LENGTH: u8 = 8;
pub const MAX_LENGTH: u8 = 64;

const LOWER: &str = "abcdefghijklmnopqrstuvwxyz";
const UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS: &str = "0123456789";
/// 不含引號、反斜線、`<`、`>`、`|`、`/`：它們在 shell、JSON、CSV、XML 裡容易出事。
const SYMBOLS: &str = "!@#$%^&*()-_=+[]{};:,.?";
/// 人眼或手動轉錄時會混淆的字元。
const LOOKALIKE: &str = "0O1lI5S2Z8B";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenOpts {
    pub length: u8,
    pub lower: bool,
    pub upper: bool,
    pub digits: bool,
    pub symbols: bool,
    pub exclude_lookalike: bool,
    pub every_class: bool,
}

impl Default for GenOpts {
    fn default() -> Self {
        GenOpts {
            length: 20,
            lower: true,
            upper: true,
            digits: true,
            symbols: true,
            exclude_lookalike: true,
            every_class: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GenError {
    #[error("no character class enabled")]
    NoClassEnabled,
    #[error("length must be between {MIN_LENGTH} and {MAX_LENGTH}")]
    LengthOutOfRange,
    /// 每類至少一個，但類別數比長度還多。
    #[error("length too short to include every enabled class")]
    TooShortForEveryClass,
    /// 排除相似字元後某個啟用的類別空了。
    #[error("a class became empty after excluding look-alike characters")]
    ClassEmptyAfterExclusion,
}

impl GenOpts {
    /// 啟用且已套用相似字元排除的類別，依固定順序。
    fn classes(&self) -> Result<Vec<Vec<char>>, GenError> {
        let mut out = Vec::new();
        for (enabled, set) in [
            (self.lower, LOWER),
            (self.upper, UPPER),
            (self.digits, DIGITS),
            (self.symbols, SYMBOLS),
        ] {
            if !enabled {
                continue;
            }
            let chars: Vec<char> = set
                .chars()
                .filter(|c| !self.exclude_lookalike || !LOOKALIKE.contains(*c))
                .collect();
            if chars.is_empty() {
                return Err(GenError::ClassEmptyAfterExclusion);
            }
            out.push(chars);
        }
        if out.is_empty() {
            return Err(GenError::NoClassEnabled);
        }
        Ok(out)
    }

    fn check(&self) -> Result<Vec<Vec<char>>, GenError> {
        if self.length < MIN_LENGTH || self.length > MAX_LENGTH {
            return Err(GenError::LengthOutOfRange);
        }
        let classes = self.classes()?;
        if self.every_class && classes.len() > self.length as usize {
            return Err(GenError::TooShortForEveryClass);
        }
        Ok(classes)
    }

    /// log2(字元集大小) × 長度，是上界：`every_class` 略微降低的熵不計入，
    /// 差距在 1 bit 以內。
    pub fn entropy_bits(&self) -> f64 {
        let Ok(classes) = self.classes() else {
            return 0.0;
        };
        let pool: usize = classes.iter().map(Vec::len).sum();
        (pool as f64).log2() * self.length as f64
    }
}

/// 產生一組密碼。
///
/// `every_class` 時每類各抽一個、補滿長度、再 Fisher-Yates 洗牌。
/// 洗牌不能省——少了它前幾個位置的類別是固定的，會洩漏結構。
pub fn generate(opts: &GenOpts, rng: &mut impl Rng) -> Result<String, GenError> {
    let classes = opts.check()?;
    let pool: Vec<char> = classes.iter().flatten().copied().collect();
    let len = opts.length as usize;

    let mut chars: Vec<char> = Vec::with_capacity(len);
    if opts.every_class {
        for class in &classes {
            chars.push(*class.choose(rng).expect("class is non-empty"));
        }
    }
    while chars.len() < len {
        chars.push(*pool.choose(rng).expect("pool is non-empty"));
    }
    chars.shuffle(rng);

    Ok(chars.into_iter().collect())
}

/// 用作業系統的 CSPRNG 產生。
pub fn generate_os(opts: &GenOpts) -> Result<String, GenError> {
    generate(opts, &mut rand::rngs::OsRng)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::StdRng;
    use rand::SeedableRng;

    fn rng() -> StdRng {
        StdRng::seed_from_u64(20260922)
    }

    #[test]
    fn honours_length() {
        for len in [MIN_LENGTH, 20, MAX_LENGTH] {
            let opts = GenOpts {
                length: len,
                ..Default::default()
            };
            let pw = generate(&opts, &mut rng()).unwrap();
            assert_eq!(pw.chars().count(), len as usize);
        }
    }

    #[test]
    fn every_class_appears_when_requested() {
        let opts = GenOpts {
            length: 8,
            ..Default::default()
        };
        // 跑多次，避免單一樣本剛好通過
        let mut r = rng();
        for _ in 0..200 {
            let pw = generate(&opts, &mut r).unwrap();
            assert!(pw.chars().any(|c| LOWER.contains(c)), "{pw}");
            assert!(pw.chars().any(|c| UPPER.contains(c)), "{pw}");
            assert!(pw.chars().any(|c| DIGITS.contains(c)), "{pw}");
            assert!(pw.chars().any(|c| SYMBOLS.contains(c)), "{pw}");
        }
    }

    #[test]
    fn excludes_lookalike_characters() {
        let opts = GenOpts {
            length: 64,
            ..Default::default()
        };
        let mut r = rng();
        for _ in 0..50 {
            let pw = generate(&opts, &mut r).unwrap();
            assert!(
                !pw.chars().any(|c| LOOKALIKE.contains(c)),
                "{pw} 含有相似字元"
            );
        }
    }

    #[test]
    fn keeps_lookalikes_when_not_excluded() {
        let opts = GenOpts {
            length: 64,
            exclude_lookalike: false,
            ..Default::default()
        };
        let mut r = rng();
        let seen_any = (0..50)
            .map(|_| generate(&opts, &mut r).unwrap())
            .any(|pw| pw.chars().any(|c| LOOKALIKE.contains(c)));
        assert!(seen_any, "50 組 64 字元密碼竟然完全沒出現相似字元");
    }

    #[test]
    fn disabled_classes_never_appear() {
        let opts = GenOpts {
            length: 32,
            symbols: false,
            digits: false,
            ..Default::default()
        };
        let mut r = rng();
        for _ in 0..50 {
            let pw = generate(&opts, &mut r).unwrap();
            assert!(!pw.chars().any(|c| SYMBOLS.contains(c)), "{pw}");
            assert!(!pw.chars().any(|c| DIGITS.contains(c)), "{pw}");
        }
    }

    /// 少了洗牌，前 N 個字元的類別會固定成類別順序。
    #[test]
    fn output_is_shuffled_not_class_ordered() {
        let opts = GenOpts {
            length: 16,
            ..Default::default()
        };
        let mut r = rng();
        let class_ordered = (0..100)
            .map(|_| generate(&opts, &mut r).unwrap())
            .filter(|pw| {
                let mut it = pw.chars();
                LOWER.contains(it.next().unwrap())
                    && UPPER.contains(it.next().unwrap())
                    && DIGITS.contains(it.next().unwrap())
                    && SYMBOLS.contains(it.next().unwrap())
            })
            .count();
        assert!(class_ordered < 5, "前四碼照類別順序出現 {class_ordered} 次");
    }

    #[test]
    fn rejects_bad_options() {
        let all_off = GenOpts {
            lower: false,
            upper: false,
            digits: false,
            symbols: false,
            ..Default::default()
        };
        assert_eq!(
            generate(&all_off, &mut rng()),
            Err(GenError::NoClassEnabled)
        );

        let short = GenOpts {
            length: MIN_LENGTH - 1,
            ..Default::default()
        };
        assert_eq!(
            generate(&short, &mut rng()),
            Err(GenError::LengthOutOfRange)
        );

        let long = GenOpts {
            length: MAX_LENGTH + 1,
            ..Default::default()
        };
        assert_eq!(generate(&long, &mut rng()), Err(GenError::LengthOutOfRange));
    }

    #[test]
    fn entropy_matches_pool_size() {
        let opts = GenOpts {
            length: 10,
            lower: true,
            upper: false,
            digits: false,
            symbols: false,
            exclude_lookalike: false,
            every_class: false,
        };
        assert!((opts.entropy_bits() - 10.0 * 26f64.log2()).abs() < 1e-9);
    }

    #[test]
    fn entropy_drops_when_lookalikes_excluded() {
        let with = GenOpts {
            exclude_lookalike: false,
            ..Default::default()
        };
        let without = GenOpts {
            exclude_lookalike: true,
            ..Default::default()
        };
        assert!(without.entropy_bits() < with.entropy_bits());
    }

    #[test]
    fn os_rng_path_works() {
        let pw = generate_os(&GenOpts::default()).unwrap();
        assert_eq!(pw.chars().count(), 20);
    }
}
