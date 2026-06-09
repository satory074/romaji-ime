//! Romaji → hiragana transliteration.
//!
//! The session stores the **raw romaji buffer** and re-runs [`convert`] over the
//! whole buffer on every keystroke. Reconverting the entire buffer (rather than
//! processing incrementally) sidesteps the classic lookahead ambiguities — e.g.
//! `honn` → ほん but `honna` → ほんな — because full right-context is always
//! available when more characters arrive.
//!
//! Rules of note:
//!   - **Sokuon (っ)**: a doubled consonant (`kk`, `tt`, `ss`, …, but not `nn`)
//!     emits っ and consumes one letter. `t`+`ch` (e.g. `matcha`) also → っ.
//!   - **ん**: `n`+`n`+(vowel|`y`) → ん from the first `n` only (so `nni`→んに);
//!     `n`+`n`+(consonant|end) → a single ん from both (so `honn`→ほん);
//!     `n`+(other consonant) → ん; `n`+(vowel|`y`) is a na-row syllable;
//!     a trailing lone `n` stays pending and is flushed to ん on commit.
//!   - Unknown characters (punctuation not in the table) pass through unchanged.
//!
//! [`convert`] returns `(kana, pending)` where `pending` is the unresolved romaji
//! tail. [`flush`] produces the final string for commit (trailing `n` → ん).

const MAX_KEY_LEN: usize = 4;

/// romaji -> hiragana. Single source of truth for both [`lookup`] and the
/// prefix check, so they can never disagree.
#[rustfmt::skip]
const TABLE: &[(&str, &str)] = &[
    // vowels
    ("a", "あ"), ("i", "い"), ("u", "う"), ("e", "え"), ("o", "お"),
    // k / g
    ("ka", "か"), ("ki", "き"), ("ku", "く"), ("ke", "け"), ("ko", "こ"),
    ("kya", "きゃ"), ("kyu", "きゅ"), ("kyo", "きょ"),
    ("ga", "が"), ("gi", "ぎ"), ("gu", "ぐ"), ("ge", "げ"), ("go", "ご"),
    ("gya", "ぎゃ"), ("gyu", "ぎゅ"), ("gyo", "ぎょ"),
    // s / z / j
    ("sa", "さ"), ("si", "し"), ("shi", "し"), ("su", "す"), ("se", "せ"), ("so", "そ"),
    ("sha", "しゃ"), ("shu", "しゅ"), ("sho", "しょ"), ("she", "しぇ"),
    ("sya", "しゃ"), ("syu", "しゅ"), ("syo", "しょ"),
    ("za", "ざ"), ("zi", "じ"), ("ji", "じ"), ("zu", "ず"), ("ze", "ぜ"), ("zo", "ぞ"),
    ("ja", "じゃ"), ("ju", "じゅ"), ("jo", "じょ"), ("je", "じぇ"),
    ("jya", "じゃ"), ("jyu", "じゅ"), ("jyo", "じょ"),
    ("zya", "じゃ"), ("zyu", "じゅ"), ("zyo", "じょ"),
    // t / d
    ("ta", "た"), ("ti", "ち"), ("chi", "ち"), ("tu", "つ"), ("tsu", "つ"), ("te", "て"), ("to", "と"),
    ("cha", "ちゃ"), ("chu", "ちゅ"), ("cho", "ちょ"), ("che", "ちぇ"),
    ("tya", "ちゃ"), ("tyu", "ちゅ"), ("tyo", "ちょ"),
    ("tsa", "つぁ"), ("tsi", "つぃ"), ("tse", "つぇ"), ("tso", "つぉ"),
    ("da", "だ"), ("di", "ぢ"), ("du", "づ"), ("de", "で"), ("do", "ど"),
    ("dya", "ぢゃ"), ("dyu", "ぢゅ"), ("dyo", "ぢょ"),
    // n
    ("na", "な"), ("ni", "に"), ("nu", "ぬ"), ("ne", "ね"), ("no", "の"),
    ("nya", "にゃ"), ("nyu", "にゅ"), ("nyo", "にょ"),
    // h / f / b / p
    ("ha", "は"), ("hi", "ひ"), ("hu", "ふ"), ("fu", "ふ"), ("he", "へ"), ("ho", "ほ"),
    ("hya", "ひゃ"), ("hyu", "ひゅ"), ("hyo", "ひょ"),
    ("fa", "ふぁ"), ("fi", "ふぃ"), ("fe", "ふぇ"), ("fo", "ふぉ"),
    ("ba", "ば"), ("bi", "び"), ("bu", "ぶ"), ("be", "べ"), ("bo", "ぼ"),
    ("bya", "びゃ"), ("byu", "びゅ"), ("byo", "びょ"),
    ("pa", "ぱ"), ("pi", "ぴ"), ("pu", "ぷ"), ("pe", "ぺ"), ("po", "ぽ"),
    ("pya", "ぴゃ"), ("pyu", "ぴゅ"), ("pyo", "ぴょ"),
    // m / y / r / w
    ("ma", "ま"), ("mi", "み"), ("mu", "む"), ("me", "め"), ("mo", "も"),
    ("mya", "みゃ"), ("myu", "みゅ"), ("myo", "みょ"),
    ("ya", "や"), ("yu", "ゆ"), ("yo", "よ"),
    ("ra", "ら"), ("ri", "り"), ("ru", "る"), ("re", "れ"), ("ro", "ろ"),
    ("rya", "りゃ"), ("ryu", "りゅ"), ("ryo", "りょ"),
    ("wa", "わ"), ("wo", "を"), ("wi", "うぃ"), ("we", "うぇ"),
    // v
    ("va", "ゔぁ"), ("vi", "ゔぃ"), ("vu", "ゔ"), ("ve", "ゔぇ"), ("vo", "ゔぉ"),
    // small kana (x / l prefix)
    ("xa", "ぁ"), ("xi", "ぃ"), ("xu", "ぅ"), ("xe", "ぇ"), ("xo", "ぉ"),
    ("la", "ぁ"), ("li", "ぃ"), ("lu", "ぅ"), ("le", "ぇ"), ("lo", "ぉ"),
    ("xya", "ゃ"), ("xyu", "ゅ"), ("xyo", "ょ"),
    ("lya", "ゃ"), ("lyu", "ゅ"), ("lyo", "ょ"),
    ("xtu", "っ"), ("xtsu", "っ"), ("ltu", "っ"), ("ltsu", "っ"),
    ("xwa", "ゎ"), ("lwa", "ゎ"),
    // punctuation
    ("-", "ー"), (".", "。"), (",", "、"), ("[", "「"), ("]", "」"),
];

fn lookup(s: &str) -> Option<&'static str> {
    TABLE.iter().find(|(k, _)| *k == s).map(|(_, v)| *v)
}

/// Is `s` a strict prefix of some table key (i.e. more typing could complete it)?
fn is_prefix_of_key(s: &str) -> bool {
    TABLE
        .iter()
        .any(|(k, _)| k.len() > s.len() && k.starts_with(s))
}

fn is_vowel(c: char) -> bool {
    matches!(c, 'a' | 'i' | 'u' | 'e' | 'o')
}

/// A consonant that can begin a syllable and thus double into a sokuon (っ).
/// Excludes vowels and `n` (which has its own handling).
fn is_sokuon_consonant(c: char) -> bool {
    c.is_ascii_alphabetic() && !is_vowel(c) && c != 'n'
}

/// Convert a raw romaji buffer into `(kana, pending)`, where `pending` is the
/// trailing romaji that has not yet resolved to kana.
pub fn convert(raw: &str) -> (String, String) {
    let chars: Vec<char> = raw.chars().collect();
    let n = chars.len();
    let mut out = String::new();
    let mut i = 0;

    while i < n {
        // Sokuon: a doubled syllable-initial consonant.
        if i + 1 < n && chars[i] == chars[i + 1] && is_sokuon_consonant(chars[i]) {
            out.push('っ');
            i += 1;
            continue;
        }
        // `t` + "ch" -> っ (e.g. "matcha" -> まっちゃ).
        if chars[i] == 't' && i + 2 < n && chars[i + 1] == 'c' && chars[i + 2] == 'h' {
            out.push('っ');
            i += 1;
            continue;
        }
        // `n` handling.
        if chars[i] == 'n' {
            let next = chars.get(i + 1).copied();
            match next {
                None => break, // lone trailing `n` -> pending
                Some('\'') => {
                    out.push('ん');
                    i += 2;
                    continue;
                }
                Some('n') => {
                    let after = chars.get(i + 2).copied();
                    if matches!(after, Some(c) if is_vowel(c) || c == 'y') {
                        // first `n` -> ん, second `n` begins the next syllable
                        out.push('ん');
                        i += 1;
                    } else {
                        // `nn` at end or before a consonant -> a single ん
                        out.push('ん');
                        i += 2;
                    }
                    continue;
                }
                Some(c) if !is_vowel(c) && c != 'y' => {
                    // `n` before another consonant -> ん
                    out.push('ん');
                    i += 1;
                    continue;
                }
                // `n` + vowel / `y` falls through to table matching (na-row, nya…).
                _ => {}
            }
        }

        // Longest table match starting at `i`.
        let maxk = std::cmp::min(MAX_KEY_LEN, n - i);
        let mut matched = false;
        for len in (1..=maxk).rev() {
            let sub: String = chars[i..i + len].iter().collect();
            if let Some(kana) = lookup(&sub) {
                out.push_str(kana);
                i += len;
                matched = true;
                break;
            }
        }
        if matched {
            continue;
        }

        // No complete match. If the tail could still complete a key, wait.
        let tail: String = chars[i..].iter().collect();
        if is_prefix_of_key(&tail) {
            break;
        }
        // Otherwise it's an unconvertible character (punctuation/unknown): pass through.
        out.push(chars[i]);
        i += 1;
    }

    let pending: String = chars[i..].iter().collect();
    (out, pending)
}

/// Produce the final string to commit. A trailing pending `n` becomes ん; any
/// other unresolved pending is appended as-is.
pub fn flush(raw: &str) -> String {
    let (mut kana, pending) = convert(raw);
    if pending == "n" {
        kana.push('ん');
    } else {
        kana.push_str(&pending);
    }
    kana
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(raw: &str) -> String {
        let (kana, pending) = convert(raw);
        format!("{kana}|{pending}")
    }

    #[test]
    fn basic_syllables() {
        assert_eq!(convert("ka").0, "か");
        assert_eq!(convert("ki").0, "き");
        assert_eq!(convert("shi").0, "し");
        assert_eq!(convert("tsu").0, "つ");
        assert_eq!(convert("a").0, "あ");
    }

    #[test]
    fn youon_digraphs() {
        assert_eq!(convert("kya").0, "きゃ");
        assert_eq!(convert("sha").0, "しゃ");
        assert_eq!(convert("cho").0, "ちょ");
        assert_eq!(convert("ryu").0, "りゅ");
    }

    #[test]
    fn the_m1_demo_word() {
        let (kana, pending) = convert("konnichiha");
        assert_eq!(kana, "こんにちは");
        assert_eq!(pending, "");
    }

    #[test]
    fn n_before_consonant_is_geminate_n() {
        assert_eq!(convert("nihongo").0, "にほんご");
        assert_eq!(convert("gunma").0, "ぐんま");
        // "sannen": さ ん ね + trailing n pending; flush completes it to 三年.
        assert_eq!(convert("sannen"), ("さんね".to_string(), "n".to_string()));
        assert_eq!(flush("sannen"), "さんねん");
    }

    #[test]
    fn nn_plus_vowel_is_n_then_na_row() {
        assert_eq!(convert("sonna").0, "そんな");
        assert_eq!(convert("minna").0, "みんな");
        assert_eq!(convert("annai").0, "あんない");
    }

    #[test]
    fn double_n_at_end_is_single_n() {
        // `honn` -> ほん (one ん), but `honna` -> ほんな (reconverting the whole buffer).
        assert_eq!(convert("honn"), ("ほん".to_string(), "".to_string()));
        assert_eq!(convert("honna").0, "ほんな");
    }

    #[test]
    fn trailing_n_stays_pending_then_resolves() {
        assert_eq!(k("hon"), "ほ|n"); // pending n
        assert_eq!(convert("hona").0, "ほな"); // n + vowel = na-row
        assert_eq!(flush("hon"), "ほん"); // committed flush turns trailing n into ん
    }

    #[test]
    fn sokuon_double_consonant() {
        assert_eq!(convert("kitte").0, "きって");
        assert_eq!(convert("assari").0, "あっさり");
        assert_eq!(convert("matcha").0, "まっちゃ"); // t + ch
        assert_eq!(convert("maccha").0, "まっちゃ"); // c + c
    }

    #[test]
    fn partial_input_stays_pending() {
        assert_eq!(k("ky"), "|ky");
        assert_eq!(k("k"), "|k");
        assert_eq!(k("sh"), "|sh");
        assert_eq!(k("ch"), "|ch");
    }

    #[test]
    fn long_vowel_and_punctuation() {
        assert_eq!(convert("ra-men"), ("らーめ".to_string(), "n".to_string())); // - -> ー
        assert_eq!(flush("ra-men"), "らーめん");
        assert_eq!(convert("a.").0, "あ。");
        assert_eq!(convert("a,i").0, "あ、い");
    }

    #[test]
    fn small_kana() {
        assert_eq!(convert("xa").0, "ぁ");
        assert_eq!(convert("xtu").0, "っ");
        assert_eq!(convert("ltsu").0, "っ");
    }

    #[test]
    fn flush_appends_incomplete_pending_as_is() {
        assert_eq!(flush("konnichiha"), "こんにちは");
        assert_eq!(flush("ky"), "ky");
        assert_eq!(flush(""), "");
    }
}
