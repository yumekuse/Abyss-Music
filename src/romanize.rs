// Line-by-line romanization: Korean → Revised Romanization and Chinese → pinyin (both local),
// Japanese → romaji (Google's reading, which handles kanji; local kana fallback when offline),
// any other non-Latin script → ASCII transliteration. Latin lines return "".
use pinyin::ToPinyin;
use wana_kana::ConvertJapanese;

fn is_kana(c: char) -> bool {
    matches!(c as u32, 0x3040..=0x30ff | 0x31f0..=0x31ff | 0xff66..=0xff9f)
}
fn is_han(c: char) -> bool {
    matches!(c as u32, 0x4e00..=0x9fff | 0x3400..=0x4dbf | 0xf900..=0xfaff | 0x20000..=0x2ffff)
}
fn is_hangul(c: char) -> bool {
    matches!(c as u32, 0xac00..=0xd7a3 | 0x1100..=0x11ff | 0x3130..=0x318f)
}
fn non_latin(s: &str) -> bool {
    s.chars().any(|c| c.is_alphabetic() && !c.is_ascii() && !matches!(c as u32, 0x00c0..=0x024f | 0x1e00..=0x1eff))
}
fn tidy(s: &str) -> String {
    let mut out = s.split_whitespace().collect::<Vec<_>>().join(" ");
    for p in [",", ".", "!", "?", "'", "’", ")", "」", "』"] {
        out = out.replace(&format!(" {p}"), p);
    }
    out
}

const L: [&str; 19] = ["g", "kk", "n", "d", "tt", "r", "m", "b", "pp", "s", "ss", "", "j", "jj", "ch", "k", "t", "p", "h"];
const V: [&str; 21] = ["a", "ae", "ya", "yae", "eo", "e", "yeo", "ye", "o", "wa", "wae", "oe", "yo", "u", "wo", "we", "wi", "yu", "eu", "ui", "i"];
const T: [&str; 28] = ["", "k", "k", "k", "n", "n", "n", "t", "l", "k", "m", "l", "l", "l", "p", "l", "m", "p", "p", "t", "t", "ng", "t", "t", "k", "t", "p", "t"];
fn korean(line: &str) -> String {
    let mut out = String::new();
    for ch in line.chars() {
        let c = ch as i64 - 0xac00;
        if !(0..=11171).contains(&c) {
            out.push(ch);
            continue;
        }
        let (l, v, t) = ((c / 588) as usize, ((c % 588) / 28) as usize, (c % 28) as usize);
        let lead = if l == 5 && (out.is_empty() || out.ends_with(char::is_whitespace)) { "r" } else { L[l] };
        out.push_str(lead);
        out.push_str(V[v]);
        out.push_str(T[t]);
    }
    tidy(&out)
}
fn chinese(line: &str) -> String {
    let mut out = String::new();
    for (ch, p) in line.chars().zip(line.to_pinyin()) {
        match p {
            Some(p) => {
                out.push(' ');
                out.push_str(p.with_tone());
                out.push(' ');
            }
            None => out.push(ch),
        }
    }
    tidy(&out)
}

pub async fn romanize_lines(http: &reqwest::Client, lines: Vec<String>) -> Vec<String> {
    let japanese = lines.iter().any(|l| l.chars().any(is_kana));
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut ja_idx: Vec<usize> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !non_latin(line) {
            out.push(String::new());
        } else if line.chars().any(is_hangul) {
            out.push(korean(line));
        } else if japanese && line.chars().any(|c| is_kana(c) || is_han(c)) {
            out.push(String::new());
            ja_idx.push(i);
        } else if line.chars().any(is_han) {
            out.push(chinese(line));
        } else {
            out.push(tidy(&any_ascii::any_ascii(line)));
        }
    }
    if !ja_idx.is_empty() {
        let ja: Vec<String> = ja_idx.iter().map(|&i| lines[i].clone()).collect();
        let google = crate::lyrics::google_roman(http, &ja, "ja").await;
        for (k, &i) in ja_idx.iter().enumerate() {
            let g = google.as_ref().map(|v| v[k].as_str()).unwrap_or("");
            out[i] = if !g.is_empty() { tidy(g) } else { tidy(&ja[k].to_romaji()) };
        }
    }
    out
}
