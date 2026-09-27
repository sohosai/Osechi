//! クレート全体で共通のエラー型。
//!
//! [`Error`] は「何をしようとして失敗したか」をメッセージに持ち、下位クレートの
//! エラーを [`std::error::Error::source`] で辿れる形で保持する。下位のエラーに
//! 文脈を付けるには [`Context::context`] を使う。
//!
//! 表示は `{}` でこの階層のメッセージのみ、`{:#}` で原因まで `: ` 区切りで連結する。

use std::fmt;

/// 既定のエラー型を [`Error`] にした `Result`。
pub type Result<T, E = Error> = std::result::Result<T, E>;

type Source = Box<dyn std::error::Error + Send + Sync + 'static>;

#[derive(Debug)]
pub struct Error {
    message: String,
    source: Option<Source>,
}

impl Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)?;
        if f.alternate() {
            let mut source = std::error::Error::source(self);
            while let Some(err) = source {
                write!(f, ": {err}")?;
                source = err.source();
            }
        }
        Ok(())
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|err| err as &(dyn std::error::Error + 'static))
    }
}

/// 失敗に「何をしようとしていたか」の文脈を付ける。
pub trait Context<T> {
    fn context(self, message: impl Into<String>) -> Result<T>;
}

impl<T, E> Context<T> for std::result::Result<T, E>
where
    E: std::error::Error + Send + Sync + 'static,
{
    fn context(self, message: impl Into<String>) -> Result<T> {
        self.map_err(|err| Error {
            message: message.into(),
            source: Some(Box::new(err)),
        })
    }
}

impl<T> Context<T> for Option<T> {
    fn context(self, message: impl Into<String>) -> Result<T> {
        self.ok_or_else(|| Error::new(message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_display_shows_only_own_message() {
        let err = Err::<(), _>(std::io::Error::other("disk full"))
            .context("failed to save")
            .unwrap_err();
        assert_eq!(err.to_string(), "failed to save");
    }

    #[test]
    fn alternate_display_chains_sources() {
        let err = Err::<(), _>(std::io::Error::other("disk full"))
            .context("failed to save")
            .context("export aborted")
            .unwrap_err();
        assert_eq!(
            format!("{err:#}"),
            "export aborted: failed to save: disk full"
        );
    }

    #[test]
    fn none_becomes_error_with_message() {
        let err = None::<()>.context("not found").unwrap_err();
        assert_eq!(format!("{err:#}"), "not found");
    }
}
