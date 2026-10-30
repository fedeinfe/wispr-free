use wf_audio::{InputDevice, rank_devices};

fn device(id: &str) -> InputDevice {
    InputDevice {
        id: id.to_owned(),
        name: format!("{id} name"),
    }
}

fn ids(devices: &[InputDevice]) -> Vec<&str> {
    devices.iter().map(|d| d.id.as_str()).collect()
}

#[test]
fn without_a_ranking_the_system_default_comes_first() {
    let available = [device("built-in"), device("usb"), device("headset")];

    let ranked = rank_devices(&available, &[], Some("usb"));

    assert_eq!(ids(&ranked), ["usb", "built-in", "headset"]);
}

#[test]
fn ranked_devices_that_are_plugged_in_come_first_in_the_users_order() {
    let available = [device("built-in"), device("usb"), device("headset")];
    let ranking = [
        "headset".to_owned(),
        "unplugged".to_owned(),
        "usb".to_owned(),
    ];

    let ranked = rank_devices(&available, &ranking, Some("built-in"));

    assert_eq!(ids(&ranked), ["headset", "usb", "built-in"]);
}

#[test]
fn when_the_top_ranked_device_is_unplugged_the_next_one_is_used() {
    let ranking = ["headset".to_owned(), "usb".to_owned()];

    let ranked = rank_devices(
        &[device("built-in"), device("usb")],
        &ranking,
        Some("built-in"),
    );

    assert_eq!(ids(&ranked)[0], "usb");
}

#[test]
fn no_devices_means_an_empty_list() {
    assert!(rank_devices(&[], &["usb".to_owned()], None).is_empty());
}
