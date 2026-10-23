//! How a file's bytes are its text: its encoding and whether it starts with a byte order mark, read as Rider reads
//! them and written back the same way.

use encoding_rs::{
    BIG5, EUC_JP, EUC_KR, Encoding, GB18030, GBK, IBM866, ISO_8859_2, ISO_8859_5, ISO_8859_7, ISO_8859_15, KOI8_R,
    KOI8_U, MACINTOSH, SHIFT_JIS, UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1250, WINDOWS_1251, WINDOWS_1252, WINDOWS_1253,
    WINDOWS_1254, WINDOWS_1255, WINDOWS_1256, WINDOWS_1257, WINDOWS_1258,
};

/// An encoding a file is read and written in, with whether it has a byte order mark.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Charset {
    pub encoding: &'static Encoding,
    pub bom: bool,
}

impl Charset {
    pub const UTF8: Charset = Charset { encoding: UTF_8, bom: false };

    /// The status bar's name for it.
    pub fn name(&self) -> &'static str {
        match (self.encoding.name(), self.bom) {
            ("UTF-8", true) => "UTF-8 BOM",
            ("UTF-16LE", _) => "UTF-16 LE",
            ("UTF-16BE", _) => "UTF-16 BE",
            (name, _) => name,
        }
    }

    /// The encoding with .editorconfig's name for it (`utf-8-bom`, `latin1`), none for one it does not know.
    pub fn from_editorconfig(value: &str) -> Option<Charset> {
        let (encoding, bom) = match value {
            "utf-8" => (UTF_8, false),
            "utf-8-bom" => (UTF_8, true),
            "utf-16le" => (UTF_16LE, true),
            "utf-16be" => (UTF_16BE, true),
            "latin1" => (WINDOWS_1252, false),
            _ => return None,
        };
        Some(Charset { encoding, bom })
    }
}

/// What the status bar's encoding menu lists: the Unicode ones and Western first, then other scripts'.
// shortcut: ISO-8859-1 and US-ASCII are read as windows-1252, as browsers read them (encoding_rs's labels);
// carry true Latin-1 once a person needs its 0x80-0x9F as controls.
pub const LISTED: &[Charset] = &[
    Charset { encoding: UTF_8, bom: false },
    Charset { encoding: UTF_8, bom: true },
    Charset { encoding: UTF_16LE, bom: true },
    Charset { encoding: UTF_16BE, bom: true },
    Charset { encoding: WINDOWS_1252, bom: false },
    Charset { encoding: ISO_8859_15, bom: false },
    Charset { encoding: WINDOWS_1250, bom: false },
    Charset { encoding: WINDOWS_1251, bom: false },
    Charset { encoding: WINDOWS_1253, bom: false },
    Charset { encoding: WINDOWS_1254, bom: false },
    Charset { encoding: WINDOWS_1255, bom: false },
    Charset { encoding: WINDOWS_1256, bom: false },
    Charset { encoding: WINDOWS_1257, bom: false },
    Charset { encoding: WINDOWS_1258, bom: false },
    Charset { encoding: ISO_8859_2, bom: false },
    Charset { encoding: ISO_8859_5, bom: false },
    Charset { encoding: ISO_8859_7, bom: false },
    Charset { encoding: KOI8_R, bom: false },
    Charset { encoding: KOI8_U, bom: false },
    Charset { encoding: IBM866, bom: false },
    Charset { encoding: MACINTOSH, bom: false },
    Charset { encoding: SHIFT_JIS, bom: false },
    Charset { encoding: EUC_JP, bom: false },
    Charset { encoding: EUC_KR, bom: false },
    Charset { encoding: GBK, bom: false },
    Charset { encoding: GB18030, bom: false },
    Charset { encoding: BIG5, bom: false },
];

/// A file's text and how it was stored: by its byte order mark, else UTF-8 where its bytes are, else `eight_bit`
/// (the .editorconfig's charset, else windows-1252, as Roslyn reads such a source file).
pub fn decode(bytes: &[u8], eight_bit: Option<Charset>) -> (String, Charset) {
    if let Some((encoding, bom)) = Encoding::for_bom(bytes) {
        let (text, _) = encoding.decode_without_bom_handling(&bytes[bom..]);
        return (text.into_owned(), Charset { encoding, bom: true });
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return (text.to_string(), Charset::UTF8);
    }
    let charset = eight_bit.filter(|c| c.encoding != UTF_8).unwrap_or(Charset { encoding: WINDOWS_1252, bom: false });
    let (text, _) = charset.encoding.decode_without_bom_handling(bytes);
    (text.into_owned(), charset)
}

/// The bytes of `text` as `charset` stores it, or the first character it cannot hold.
pub fn encode(text: &str, charset: Charset) -> Result<Vec<u8>, char> {
    let mut bytes = Vec::with_capacity(text.len() + 3);
    let units = |be: bool| text.encode_utf16().flat_map(move |u| if be { u.to_be_bytes() } else { u.to_le_bytes() });
    match charset.encoding.name() {
        "UTF-8" => {
            if charset.bom {
                bytes.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
            }
            bytes.extend_from_slice(text.as_bytes());
        }
        // encoding_rs writes no UTF-16 of its own; it is the text's own units, after its mark.
        "UTF-16LE" => {
            bytes.extend_from_slice(&[0xFF, 0xFE]);
            bytes.extend(units(false));
        }
        "UTF-16BE" => {
            bytes.extend_from_slice(&[0xFE, 0xFF]);
            bytes.extend(units(true));
        }
        _ => {
            let (encoded, _, unmappable) = charset.encoding.encode(text);
            if unmappable {
                let mut one = [0u8; 4];
                let lost = text.chars().find(|c| charset.encoding.encode(c.encode_utf8(&mut one)).2);
                return Err(lost.unwrap_or('\u{FFFD}'));
            }
            bytes.extend_from_slice(&encoded);
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_reads_back_as_it_was_stored_in_each_encoding() {
        for (bytes, name) in [
            (&b"\xEF\xBB\xBFclass C {}"[..], "UTF-8 BOM"),
            (&b"\xFF\xFEc\0"[..], "UTF-16 LE"),
            (&b"\xFE\xFF\0c"[..], "UTF-16 BE"),
            (&b"caf\xE9"[..], "windows-1252"),
            (&b"plain"[..], "UTF-8"),
        ] {
            let (text, charset) = decode(bytes, None);
            assert_eq!(charset.name(), name, "{bytes:?} was read in the wrong encoding");
            assert_eq!(encode(&text, charset).unwrap(), bytes, "{name} was not written back byte for byte");
        }
        assert_eq!(decode(b"caf\xE9", None).0, "café");
    }

    #[test]
    fn a_character_the_encoding_cannot_hold_is_named_rather_than_written() {
        let latin = Charset { encoding: WINDOWS_1252, bom: false };
        assert_eq!(encode("a 😀 b", latin), Err('😀'), "a character windows-1252 cannot hold was written anyway");
    }
}
