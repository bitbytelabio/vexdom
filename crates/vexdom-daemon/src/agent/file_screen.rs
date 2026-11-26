//! What an attached file is, judged by its bytes, and whether the agent can open it: everything
//! decided before an analyst is started (`file-screen.ts`).

use super::file_report::{FileKind, MAX_STORED_FILE_BYTES, file_limit};

/// What the browser said about the file.
pub struct ScreenedFile<'a> {
    pub name: &'a str,
    pub mime: &'a str,
    /// What the browser measured; the bytes win when they arrived.
    pub size: u64,
}

/// The agent that would read the file.
pub struct Reader<'a> {
    pub label: &'a str,
    pub opens: &'a [FileKind],
    /// The agents that can open a kind this one cannot, to name in the rejection.
    pub openers: &'a dyn Fn(FileKind) -> Vec<String>,
}

/// Why a file is not read: `EMPTY`, `FILE_TOO_LARGE` or `UNSUPPORTED_TYPE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused {
    pub code: &'static str,
    pub message: String,
    pub kind: Option<FileKind>,
}

const SAMPLE_BYTES: usize = 64 * 1024;
const MB: u64 = 1024 * 1024;
const ZIP: &[u8] = b"PK\x03\x04";

/// `screenFile`: the kind the bytes are and the extension they earn, or why the file is not read.
pub fn screen_file(
    file: &ScreenedFile<'_>,
    bytes: &[u8],
    reader: &Reader<'_>,
) -> Result<(FileKind, &'static str), Refused> {
    let length = bytes.len() as u64;
    let size = file.size.max(length);
    let name = format!("“{}”", file.name);
    if size > MAX_STORED_FILE_BYTES {
        return Err(Refused {
            code: "FILE_TOO_LARGE",
            message: format!(
                "{name} is {}; files over {} are not read.",
                megabytes(size),
                megabytes(MAX_STORED_FILE_BYTES)
            ),
            kind: None,
        });
    }
    if bytes.is_empty() {
        return Err(Refused {
            code: "EMPTY",
            message: format!("{name} is empty."),
            kind: None,
        });
    }

    let Some(kind) = kind_of(bytes) else {
        return Err(Refused {
            code: "UNSUPPORTED_TYPE",
            message: format!(
                "{name} is {}. Vexdom reads text, PDF and image files.",
                describe_binary(bytes, file.mime)
            ),
            kind: None,
        });
    };

    if length > file_limit(kind) {
        return Err(Refused {
            code: "FILE_TOO_LARGE",
            message: format!(
                "{name} is {}; {} files over {} are not read.",
                megabytes(length),
                kind_name(kind),
                megabytes(file_limit(kind))
            ),
            kind: Some(kind),
        });
    }

    if !reader.opens.contains(&kind) {
        let openers = (reader.openers)(kind);
        let switch = if openers.is_empty() {
            String::new()
        } else {
            format!(" Switch to {} to have it read.", openers.join(" or "))
        };
        return Err(Refused {
            code: "UNSUPPORTED_TYPE",
            message: format!(
                "{} cannot open {} here.{switch}",
                reader.label,
                kind_plural(kind)
            ),
            kind: Some(kind),
        });
    }
    Ok((kind, extension_of(bytes, kind)))
}

fn kind_name(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Text => "text",
        FileKind::Pdf => "PDF",
        FileKind::Image => "image",
    }
}

fn kind_plural(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Text => "text files",
        FileKind::Pdf => "PDFs",
        FileKind::Image => "images",
    }
}

/// The extension the bytes earn. An agent picks how to open a file by its name, so the name must
/// not lie.
pub fn extension_of(bytes: &[u8], kind: FileKind) -> &'static str {
    match kind {
        FileKind::Text => "txt",
        FileKind::Pdf => "pdf",
        FileKind::Image if bytes.starts_with(b"GIF8") => "gif",
        FileKind::Image if bytes.starts_with(b"RIFF") => "webp",
        FileKind::Image if bytes.first() == Some(&0xff) => "jpg",
        FileKind::Image => "png",
    }
}

/// `kindOf`: what the bytes are, whatever the file was called.
pub fn kind_of(bytes: &[u8]) -> Option<FileKind> {
    if bytes.starts_with(b"%PDF-") {
        return Some(FileKind::Pdf);
    }
    if is_image(bytes) {
        return Some(FileKind::Image);
    }
    if bytes.starts_with(ZIP) {
        return None;
    }
    is_text(bytes).then_some(FileKind::Text)
}

fn is_image(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])
        || bytes.starts_with(&[0xff, 0xd8, 0xff])
        || bytes.starts_with(b"GIF87a")
        || bytes.starts_with(b"GIF89a")
        || (bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP".as_slice()))
}

/// UTF-8 with no NUL in its first 64 KiB; a character the sample cuts in two still counts.
fn is_text(bytes: &[u8]) -> bool {
    let sample = &bytes[..bytes.len().min(SAMPLE_BYTES)];
    if sample.contains(&0) {
        return false;
    }
    match std::str::from_utf8(sample) {
        Ok(_) => true,
        // `error_len` is `None` only for a sequence the end of the sample cut short.
        Err(error) => error.error_len().is_none(),
    }
}

fn describe_binary(bytes: &[u8], mime: &str) -> String {
    if bytes.starts_with(ZIP) {
        return "a ZIP archive (Word, Excel and PowerPoint files are ZIP archives too)".into();
    }
    if mime.is_empty() {
        "a binary file".into()
    } else {
        format!("a binary file ({mime})")
    }
}

/// `1.5 MB`, `5 MB`, `42 MB`: one decimal below ten (a tie rounding up, as `toFixed` does, and a
/// trailing `.0` dropped), whole megabytes from there.
fn megabytes(bytes: u64) -> String {
    if bytes < 10 * MB {
        let tenths = (bytes * 10 + MB / 2) / MB;
        if tenths.is_multiple_of(10) {
            format!("{} MB", tenths / 10)
        } else {
            format!("{}.{} MB", tenths / 10, tenths % 10)
        }
    } else {
        format!("{} MB", bytes / MB + u64::from(bytes % MB >= MB / 2))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = &[
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0x0d,
    ];
    const JPEG: &[u8] = &[0xff, 0xd8, 0xff, 0xe0, 0, 0x10];
    const PDF: &[u8] = "%PDF-1.7\n%âãÏÓ\n1 0 obj".as_bytes();
    const WEBP: &[u8] = b"RIFF\0\0\0\0WEBPVP8 ";
    const ZIP_FILE: &[u8] = &[0x50, 0x4b, 0x03, 0x04, 0x14, 0, 0, 0];
    const GIF: &[u8] = b"GIF89a\x01\x00";

    /// What a reader opens, and who it names for the rest.
    struct Agent {
        label: &'static str,
        opens: &'static [FileKind],
        openers: fn(FileKind) -> Vec<String>,
    }

    const TEXT_ONLY: Agent = Agent {
        label: "Mistral Vibe",
        opens: &[FileKind::Text],
        openers: |_| vec!["Claude Code".to_owned()],
    };

    const EVERYTHING: Agent = Agent {
        label: "Claude Code",
        opens: &[FileKind::Text, FileKind::Pdf, FileKind::Image],
        openers: |_| Vec::new(),
    };

    fn screen_as(
        bytes: &[u8],
        agent: &Agent,
        name: &str,
        mime: &str,
    ) -> Result<(FileKind, &'static str), Refused> {
        let file = ScreenedFile {
            name,
            mime,
            size: bytes.len() as u64,
        };
        let reader = Reader {
            label: agent.label,
            opens: agent.opens,
            openers: &agent.openers,
        };
        screen_file(&file, bytes, &reader)
    }

    fn screen(bytes: &[u8], name: &str, mime: &str) -> Result<(FileKind, &'static str), Refused> {
        screen_as(bytes, &EVERYTHING, name, mime)
    }

    fn refused(code: &'static str, message: &str, kind: Option<FileKind>) -> Refused {
        Refused {
            code,
            message: message.to_owned(),
            kind,
        }
    }

    #[test]
    fn what_kind_of_file_it_is() {
        let cases: [(&str, &[u8], Option<FileKind>); 10] = [
            ("a PDF", PDF, Some(FileKind::Pdf)),
            ("a PNG", PNG, Some(FileKind::Image)),
            ("a JPEG", JPEG, Some(FileKind::Image)),
            ("a GIF", GIF, Some(FileKind::Image)),
            ("a WebP", WEBP, Some(FileKind::Image)),
            (
                "CSV",
                b"date,amount\n2026-08-01,12.50\n",
                Some(FileKind::Text),
            ),
            (
                "UTF-8 prose with accents",
                "Crème brûlée — 3 €".as_bytes(),
                Some(FileKind::Text),
            ),
            ("a ZIP", ZIP_FILE, None),
            ("bytes with a NUL in them", &[0x41, 0x00, 0x42], None),
            ("invalid UTF-8", &[0x41, 0xc3, 0x28, 0x42], None),
        ];
        for (what, bytes, kind) in cases {
            assert_eq!(kind_of(bytes), kind, "{what}");
        }
    }

    #[test]
    fn a_multi_byte_character_cut_by_the_sample_boundary_is_still_text() {
        let mut bytes = vec![0x61; 64 * 1024 - 1];
        bytes.extend_from_slice("é".as_bytes());
        assert_eq!(kind_of(&bytes), Some(FileKind::Text));
    }

    #[test]
    fn the_bytes_decide_not_the_name_or_type_the_browser_gave_it() {
        assert_eq!(
            [
                screen(PNG, "notes.txt", "text/plain"),
                screen(b"plain words", "photo.png", "image/png")
            ],
            [Ok((FileKind::Image, "png")), Ok((FileKind::Text, "txt"))]
        );
    }

    #[test]
    fn each_kind_is_named_for_what_it_is_whatever_it_was_called() {
        let cases: [(&[u8], &str); 6] = [
            (PDF, "pdf"),
            (PNG, "png"),
            (JPEG, "jpg"),
            (GIF, "gif"),
            (WEBP, "webp"),
            (b"plain words", "txt"),
        ];
        for (bytes, extension) in cases {
            let kind = kind_of(bytes).unwrap_or(FileKind::Text);
            assert_eq!(extension_of(bytes, kind), extension);
        }
    }

    #[test]
    fn an_empty_file_is_rejected() {
        assert_eq!(
            screen(b"", "blank.txt", ""),
            Err(refused("EMPTY", "“blank.txt” is empty.", None))
        );
    }

    #[test]
    fn a_file_over_the_storage_cap_arrives_as_a_size_with_no_bytes() {
        let file = ScreenedFile {
            name: "backup.sql",
            mime: "",
            size: 42 * MB,
        };
        let reader = Reader {
            label: EVERYTHING.label,
            opens: EVERYTHING.opens,
            openers: &EVERYTHING.openers,
        };
        assert_eq!(
            screen_file(&file, b"", &reader),
            Err(refused(
                "FILE_TOO_LARGE",
                "“backup.sql” is 42 MB; files over 10 MB are not read.",
                None
            ))
        );
    }

    #[test]
    fn an_archive_is_named_as_one() {
        assert_eq!(
            screen(ZIP_FILE, "report.docx", ""),
            Err(refused(
                "UNSUPPORTED_TYPE",
                "“report.docx” is a ZIP archive (Word, Excel and PowerPoint files are ZIP archives too). Vexdom reads text, PDF and image files.",
                None
            ))
        );
    }

    #[test]
    fn any_other_binary_comes_with_the_type_the_browser_gave_it() {
        assert_eq!(
            screen(
                &[0x7f, 0x45, 0x4c, 0x46, 0x00],
                "tool",
                "application/octet-stream"
            ),
            Err(refused(
                "UNSUPPORTED_TYPE",
                "“tool” is a binary file (application/octet-stream). Vexdom reads text, PDF and image files.",
                None
            ))
        );
    }

    #[test]
    fn a_kind_over_its_own_limit_below_the_storage_cap() {
        let huge = vec![0x61; 6 * 1024 * 1024];
        assert_eq!(
            screen(&huge, "huge.log", ""),
            Err(refused(
                "FILE_TOO_LARGE",
                "“huge.log” is 6 MB; text files over 5 MB are not read.",
                Some(FileKind::Text)
            ))
        );
    }

    #[test]
    fn a_pdf_for_an_agent_that_cannot_open_one_names_the_agents_that_can() {
        assert_eq!(
            screen_as(PDF, &TEXT_ONLY, "invoice.pdf", ""),
            Err(refused(
                "UNSUPPORTED_TYPE",
                "Mistral Vibe cannot open PDFs here. Switch to Claude Code to have it read.",
                Some(FileKind::Pdf)
            ))
        );
    }

    #[test]
    fn text_reaches_every_agent() {
        assert_eq!(
            screen_as(b"hello", &TEXT_ONLY, "upload", ""),
            Ok((FileKind::Text, "txt"))
        );
    }

    #[test]
    fn sizes_read_as_to_fixed_prints_them() {
        let cases = [
            (0, "0 MB"),
            (MB / 4, "0.3 MB"),
            (MB + MB / 2, "1.5 MB"),
            (5 * MB, "5 MB"),
            (5 * MB + MB / 4, "5.3 MB"),
            (10 * MB - 1, "10 MB"),
            (10 * MB + MB / 2, "11 MB"),
            (u64::MAX, "17592186044416 MB"),
        ];
        for (bytes, text) in cases {
            assert_eq!(megabytes(bytes), text, "{bytes} bytes");
        }
    }
}
