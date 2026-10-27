// SPDX-License-Identifier: GPL-3.0-only

//! Typed, provider-neutral backend errors.
//!
//! Service error *messages* are dropped: they may contain labels or other
//! sensitive text. Only the D-Bus error *name* is kept for unknown errors.

use super::crypto::CryptoError;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("no Secret Service is available on the session bus")]
    NoService,
    #[error("the Secret Service could not be started")]
    ActivationFailed,
    #[error("permission denied")]
    PermissionDenied,
    #[error("the service does not offer encrypted secret transfer")]
    EncryptionUnavailable,
    #[error("the object is locked")]
    Locked,
    #[error("the object no longer exists")]
    NoSuchObject,
    #[error("the object belongs to a previous connection")]
    Stale,
    #[error("the prompt was dismissed")]
    PromptDismissed,
    #[error("disconnected from the Secret Service")]
    Disconnected,
    #[error("the operation is not supported by the service")]
    Unsupported,
    #[error("the keyring is not empty")]
    NotEmpty,
    #[error("the service accepted the request but did not apply it")]
    NotApplied,
    #[error("the service did not respond in time")]
    Timeout,
    #[error("the secret transfer session is no longer valid")]
    SessionLost,
    #[error("unexpected reply from the service")]
    Protocol,
    #[error("service error {name}")]
    Service { name: String },
    #[error("encryption error: {0}")]
    Crypto(#[from] CryptoError),
}

impl Error {
    fn from_error_name(name: &str) -> Self {
        const DBUS: &str = "org.freedesktop.DBus.Error.";
        const SECRET: &str = "org.freedesktop.Secret.Error.";

        if let Some(short) = name.strip_prefix(SECRET) {
            return match short {
                "IsLocked" => Self::Locked,
                "NoSuchObject" => Self::NoSuchObject,
                "NoSession" => Self::SessionLost,
                _ => Self::Service {
                    name: name.to_owned(),
                },
            };
        }
        if let Some(short) = name.strip_prefix(DBUS) {
            if short.starts_with("Spawn.") {
                return Self::ActivationFailed;
            }
            return match short {
                "ServiceUnknown" | "NameHasNoOwner" => Self::NoService,
                "AccessDenied" | "AuthFailed" | "InteractiveAuthorizationRequired" => {
                    Self::PermissionDenied
                }
                "UnknownMethod" | "UnknownInterface" | "UnknownProperty" | "NotSupported" => {
                    Self::Unsupported
                }
                "UnknownObject" => Self::NoSuchObject,
                "NoReply" | "Timeout" | "TimedOut" => Self::Timeout,
                "Disconnected" | "NoServer" | "NoNetwork" => Self::Disconnected,
                _ => Self::Service {
                    name: name.to_owned(),
                },
            };
        }
        Self::Service {
            name: name.to_owned(),
        }
    }
}

impl From<zbus::Error> for Error {
    fn from(err: zbus::Error) -> Self {
        use zbus::DBusError;
        match err {
            zbus::Error::MethodError(name, _message, _) => Self::from_error_name(name.as_str()),
            zbus::Error::FDO(fdo) => match *fdo {
                zbus::fdo::Error::ZBus(inner) => inner.into(),
                fdo => Self::from_error_name(fdo.name().as_str()),
            },
            zbus::Error::InputOutput(io) if io.kind() == std::io::ErrorKind::TimedOut => {
                Self::Timeout
            }
            zbus::Error::InputOutput(_) | zbus::Error::Handshake(_) | zbus::Error::Address(_) => {
                Self::Disconnected
            }
            _ => Self::Protocol,
        }
    }
}

impl From<zbus::fdo::Error> for Error {
    fn from(err: zbus::fdo::Error) -> Self {
        zbus::Error::FDO(Box::new(err)).into()
    }
}

impl From<zbus::zvariant::Error> for Error {
    fn from(_: zbus::zvariant::Error) -> Self {
        Self::Protocol
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_well_known_names() {
        let cases = [
            ("org.freedesktop.Secret.Error.IsLocked", Error::Locked),
            (
                "org.freedesktop.Secret.Error.NoSuchObject",
                Error::NoSuchObject,
            ),
            (
                "org.freedesktop.DBus.Error.ServiceUnknown",
                Error::NoService,
            ),
            (
                "org.freedesktop.DBus.Error.Spawn.ChildExited",
                Error::ActivationFailed,
            ),
            (
                "org.freedesktop.DBus.Error.AccessDenied",
                Error::PermissionDenied,
            ),
            (
                "org.freedesktop.DBus.Error.UnknownMethod",
                Error::Unsupported,
            ),
            ("org.freedesktop.DBus.Error.NoReply", Error::Timeout),
        ];
        for (name, expected) in cases {
            assert_eq!(Error::from_error_name(name), expected, "{name}");
        }
        assert_eq!(
            Error::from_error_name("org.example.Custom"),
            Error::Service {
                name: "org.example.Custom".into()
            }
        );
    }
}
