//! RPC arms in the `bcachefs.*` domain.
//!
//! On this btrfs fork these diagnostics are unsupported (or thin-wrapped
//! to `btrfs filesystem usage` via `FilesystemService::bcachefs_usage`).

#![allow(unused_imports, unused_variables)]

use nasty_common::{Request, Response};

use super::*;
use crate::AppState;
use crate::auth::Session;

const UNSUPPORTED_TOP: &str = "bcachefs.top is not supported on btrfs builds";
const UNSUPPORTED_TIMESTATS: &str = "bcachefs.timestats is not supported on btrfs builds";

pub(super) async fn try_route(
    req: &Request,
    state: &AppState,
    _session: &Session,
) -> Option<Response> {
    Some(match req.method.as_str() {
        "bcachefs.usage" => match require_str(req, "name") {
            Ok(name) => match state.filesystems.bcachefs_usage(name).await {
                Ok(v) => ok(req, v),
                Err(e) => err(req, e),
            },
            Err(r) => r,
        },
        "bcachefs.top" => err(req, UNSUPPORTED_TOP),
        "bcachefs.timestats" => err(req, UNSUPPORTED_TIMESTATS),
        _ => return None,
    })
}
