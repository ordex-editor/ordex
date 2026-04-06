//! Swap-file header parsing and serialization.

use std::io::{self, BufRead, Write};
use std::path::PathBuf;

const SWAP_MAGIC: &str = "ordex-swap-v1";

/// Metadata stored in the `ordex-swap-v1` header block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SwapMeta {
    pub(crate) pid: u32,
    pub(crate) hostname: String,
    pub(crate) original_path: PathBuf,
    pub(crate) opened_at: u64,
    pub(crate) last_refreshed_at: u64,
}

impl SwapMeta {
    /// Write the swap header to `writer` without writing the body content.
    pub(crate) fn write_header<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        writer.write_all(SWAP_MAGIC.as_bytes())?;
        writer.write_all(b"\n")?;
        writer.write_all(format!("pid={}\n", self.pid).as_bytes())?;
        writer.write_all(format!("hostname={}\n", self.hostname).as_bytes())?;
        writer.write_all(format!("original_path={}\n", self.original_path.display()).as_bytes())?;
        writer.write_all(format!("opened_at={}\n", self.opened_at).as_bytes())?;
        writer.write_all(format!("last_refreshed_at={}\n", self.last_refreshed_at).as_bytes())?;
        writer.write_all(b"\n")?;
        Ok(())
    }

    /// Read and parse one swap header, leaving `reader` at the start of the body.
    pub(crate) fn read_header<R: BufRead>(reader: &mut R) -> io::Result<Self> {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        if line.is_empty() {
            return Err(invalid_data("swap file ended before the magic header"));
        }
        if trim_line_ending(&line) != SWAP_MAGIC {
            return Err(invalid_data("swap file has an invalid magic header"));
        }

        let mut pid = None;
        let mut hostname = None;
        let mut original_path = None;
        let mut opened_at = None;
        let mut last_refreshed_at = None;

        loop {
            line.clear();
            let bytes_read = reader.read_line(&mut line)?;
            if bytes_read == 0 {
                return Err(invalid_data(
                    "swap file ended before the header/body delimiter",
                ));
            }
            let trimmed = trim_line_ending(&line);
            if trimmed.is_empty() {
                break;
            }
            let Some((key, value)) = trimmed.split_once('=') else {
                continue;
            };

            // Unknown keys are ignored so newer writers can extend the header
            // without breaking older readers.
            match key {
                "pid" => {
                    pid = Some(
                        value
                            .parse::<u32>()
                            .map_err(|_| invalid_data("swap pid must be a decimal integer"))?,
                    );
                }
                "hostname" => hostname = Some(value.to_string()),
                "original_path" => {
                    let path = PathBuf::from(value);
                    if !path.is_absolute() {
                        return Err(invalid_data("swap original_path must be absolute"));
                    }
                    original_path = Some(path);
                }
                "opened_at" => {
                    opened_at =
                        Some(value.parse::<u64>().map_err(|_| {
                            invalid_data("swap opened_at must be a decimal timestamp")
                        })?);
                }
                "last_refreshed_at" => {
                    last_refreshed_at = Some(value.parse::<u64>().map_err(|_| {
                        invalid_data("swap last_refreshed_at must be a decimal timestamp")
                    })?);
                }
                _ => {}
            }
        }

        Ok(Self {
            pid: pid.ok_or_else(|| invalid_data("swap header is missing `pid`"))?,
            hostname: hostname.ok_or_else(|| invalid_data("swap header is missing `hostname`"))?,
            original_path: original_path
                .ok_or_else(|| invalid_data("swap header is missing `original_path`"))?,
            opened_at: opened_at
                .ok_or_else(|| invalid_data("swap header is missing `opened_at`"))?,
            last_refreshed_at: last_refreshed_at
                .ok_or_else(|| invalid_data("swap header is missing `last_refreshed_at`"))?,
        })
    }
}

/// Remove one trailing `\n` or `\r\n` sequence from a header line.
fn trim_line_ending(line: &str) -> &str {
    line.strip_suffix('\n')
        .unwrap_or(line)
        .strip_suffix('\r')
        .unwrap_or(line.strip_suffix('\n').unwrap_or(line))
}

/// Build one `InvalidData` error for malformed swap files.
fn invalid_data(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufReader, Cursor};

    /// Build a representative swap metadata value for parsing and serialization tests.
    fn sample_meta() -> SwapMeta {
        SwapMeta {
            pid: 42,
            hostname: "example-host".to_string(),
            original_path: PathBuf::from("/tmp/demo.txt"),
            opened_at: 100,
            last_refreshed_at: 200,
        }
    }

    #[test]
    fn writes_header_in_expected_wire_format() {
        let mut bytes = Vec::new();
        sample_meta()
            .write_header(&mut bytes)
            .expect("write header");
        assert_eq!(
            String::from_utf8(bytes).expect("utf8 header"),
            "ordex-swap-v1\npid=42\nhostname=example-host\noriginal_path=/tmp/demo.txt\nopened_at=100\nlast_refreshed_at=200\n\n"
        );
    }

    #[test]
    fn reads_header_and_leaves_body_available() {
        let data = "ordex-swap-v1
pid=42
hostname=example-host
original_path=/tmp/demo.txt
opened_at=100
last_refreshed_at=200

body text";
        let mut reader = BufReader::new(Cursor::new(data.as_bytes()));
        let meta = SwapMeta::read_header(&mut reader).expect("read header");
        assert_eq!(meta, sample_meta());

        let mut body = String::new();
        std::io::Read::read_to_string(&mut reader, &mut body).expect("read body");
        assert_eq!(body, "body text");
    }

    #[test]
    fn accepts_out_of_order_keys_and_unknown_extensions() {
        let data = "ordex-swap-v1
hostname=example-host
future=value
last_refreshed_at=200
opened_at=100
original_path=/tmp/demo.txt
pid=42

";
        let mut reader = BufReader::new(Cursor::new(data.as_bytes()));
        let meta = SwapMeta::read_header(&mut reader).expect("read header");
        assert_eq!(meta, sample_meta());
    }

    #[test]
    fn rejects_missing_magic_header() {
        let data = "wrong\n\n";
        let error =
            SwapMeta::read_header(&mut BufReader::new(Cursor::new(data))).expect_err("reject");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn rejects_relative_original_path() {
        let data = "ordex-swap-v1
pid=42
hostname=example-host
original_path=demo.txt
opened_at=100
last_refreshed_at=200

";
        let error =
            SwapMeta::read_header(&mut BufReader::new(Cursor::new(data))).expect_err("reject");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn rejects_header_without_delimiter() {
        let data = "ordex-swap-v1
pid=42
hostname=example-host
original_path=/tmp/demo.txt
opened_at=100
last_refreshed_at=200
";
        let error =
            SwapMeta::read_header(&mut BufReader::new(Cursor::new(data))).expect_err("reject");
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
