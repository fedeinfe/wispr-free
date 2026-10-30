//! Starting and stopping real key capture. Machines without the permission (CI, mostly) get the
//! permission error instead, which is the other half of the contract.

use wf_platform::keys::{self, KeysError};

#[test]
fn capture_stops_cleanly_or_reports_the_missing_permission() {
    match keys::listen(|_| false) {
        Ok(listener) => drop(listener),
        Err(KeysError::Permission(message)) => assert!(!message.is_empty()),
        Err(error) => panic!("{error}"),
    }
}
