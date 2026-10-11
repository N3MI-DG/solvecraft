//! Which HID interfaces are 3D mice.

/// 3Dconnexion's USB vendor id.
pub const VENDOR_3DCONNEXION: u16 = 0x256F;
/// Logitech's USB vendor id (3Dconnexion devices made before 2011 carry it).
pub const VENDOR_LOGITECH: u16 = 0x046D;
/// HID usage page Generic Desktop.
pub const USAGE_PAGE_GENERIC_DESKTOP: u16 = 0x01;
/// HID usage Multi-axis Controller (on the Generic Desktop page): what a 3D mouse declares itself.
pub const USAGE_MULTI_AXIS: u16 = 0x08;

/// Known 3D mice: (vendor id, product id, model).
const MODELS: &[(u16, u16, &str)] = &[
    (VENDOR_LOGITECH, 0xC603, "SpaceMouse Plus XT"),
    (VENDOR_LOGITECH, 0xC605, "CADman"),
    (VENDOR_LOGITECH, 0xC606, "SpaceMouse Classic"),
    (VENDOR_LOGITECH, 0xC621, "Spaceball 5000"),
    (VENDOR_LOGITECH, 0xC623, "SpaceTraveler"),
    (VENDOR_LOGITECH, 0xC625, "SpacePilot"),
    (VENDOR_LOGITECH, 0xC626, "SpaceNavigator"),
    (VENDOR_LOGITECH, 0xC627, "SpaceExplorer"),
    (VENDOR_LOGITECH, 0xC628, "SpaceNavigator for Notebooks"),
    (VENDOR_LOGITECH, 0xC629, "SpacePilot Pro"),
    (VENDOR_LOGITECH, 0xC62B, "SpaceMouse Pro"),
    (VENDOR_3DCONNEXION, 0xC62E, "SpaceMouse Wireless (cabled)"),
    (VENDOR_3DCONNEXION, 0xC62F, "SpaceMouse Wireless (receiver)"),
    (VENDOR_3DCONNEXION, 0xC631, "SpaceMouse Pro Wireless (cabled)"),
    (VENDOR_3DCONNEXION, 0xC632, "SpaceMouse Pro Wireless (receiver)"),
    (VENDOR_3DCONNEXION, 0xC633, "SpaceMouse Enterprise"),
    (VENDOR_3DCONNEXION, 0xC635, "SpaceMouse Compact"),
    (VENDOR_3DCONNEXION, 0xC638, "SpaceMouse Pro Wireless (Bluetooth)"),
    (VENDOR_3DCONNEXION, 0xC63A, "SpaceMouse Wireless (Bluetooth)"),
    (VENDOR_3DCONNEXION, 0xC641, "SpaceMouse"),
    (VENDOR_3DCONNEXION, 0xC652, "3Dconnexion Universal Receiver"),
];

/// The model name of a known 3D mouse.
pub fn model_name(vendor_id: u16, product_id: u16) -> Option<&'static str> {
    MODELS.iter().find(|(v, p, _)| *v == vendor_id && *p == product_id).map(|(_, _, name)| *name)
}

/// Whether a HID interface is a 3D mouse. Any interface that declares itself a multi-axis
/// controller is one, from any maker; when the platform reports no usage (`usage_page` 0), a
/// known model is. An interface that declares another usage is not, even on a known device: a
/// receiver also carries mice and keyboards.
pub fn is_space_mouse(vendor_id: u16, product_id: u16, usage_page: u16, usage: u16) -> bool {
    if usage_page == 0 {
        return model_name(vendor_id, product_id).is_some();
    }
    usage_page == USAGE_PAGE_GENERIC_DESKTOP && usage == USAGE_MULTI_AXIS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_axis_controllers_are_space_mice() {
        // SpaceMouse Wireless over Bluetooth, as Windows lists it.
        assert!(is_space_mouse(0x256F, 0xC63A, 0x01, 0x08));
        // A model or maker not in the table still counts by its usage.
        assert!(is_space_mouse(0x1234, 0x5678, 0x01, 0x08));
    }

    #[test]
    fn other_interfaces_are_not() {
        // The Universal Receiver's mouse interface (a CadMouse paired to it).
        assert!(!is_space_mouse(0x256F, 0xC652, 0x01, 0x02));
        // A Logitech receiver's keyboard, and a headset's vendor page.
        assert!(!is_space_mouse(0x046D, 0xC52B, 0x01, 0x06));
        assert!(!is_space_mouse(0x1038, 0x12E0, 0xFFC0, 0x01));
    }

    #[test]
    fn without_a_usage_only_known_models_count() {
        assert!(is_space_mouse(0x046D, 0xC626, 0, 0));
        assert!(!is_space_mouse(0x046D, 0xC52B, 0, 0));
        // A CadMouse (an ordinary mouse) is not in the table.
        assert!(!is_space_mouse(0x256F, 0xC651, 0, 0));
    }

    #[test]
    fn models_are_named() {
        assert_eq!(model_name(0x256F, 0xC63A), Some("SpaceMouse Wireless (Bluetooth)"));
        assert_eq!(model_name(0x046D, 0xC626), Some("SpaceNavigator"));
        assert_eq!(model_name(0x256F, 0x0000), None);
        let mut ids: Vec<(u16, u16)> = MODELS.iter().map(|(v, p, _)| (*v, *p)).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), MODELS.len(), "each model once");
    }
}
