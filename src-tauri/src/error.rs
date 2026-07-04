#[derive(Debug, thiserror::Error)]
pub enum WardError {
    #[error("not found: {0}")]
    NotFound(String),
    #[error("path escaped home: {0}")]
    PathEscaped(String),
    #[error("harness unavailable: {0}")]
    HarnessUnavailable(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

#[derive(serde::Serialize)]
#[serde(tag = "kind", content = "message")]
#[serde(rename_all = "camelCase")]
enum ErrorKind {
    NotFound(String),
    PathEscaped(String),
    HarnessUnavailable(String),
    Io(String),
}

impl serde::Serialize for WardError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        let message = self.to_string();
        let kind = match self {
            WardError::NotFound(_) => ErrorKind::NotFound(message),
            WardError::PathEscaped(_) => ErrorKind::PathEscaped(message),
            WardError::HarnessUnavailable(_) => ErrorKind::HarnessUnavailable(message),
            WardError::Io(_) => ErrorKind::Io(message),
        };
        kind.serialize(serializer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_kind_and_message() {
        let e = WardError::HarnessUnavailable("codex".into());
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(json, "{\"kind\":\"harnessUnavailable\",\"message\":\"harness unavailable: codex\"}");
    }
}
