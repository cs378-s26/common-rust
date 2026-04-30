#![no_std]
#![no_main]
#![feature(custom_test_frameworks)]
#![test_runner(kernel_common::test_runner)]

kernel_common::integration_test!({
    extern crate alloc;

    use kernel_common::{
        devices::usb::xhci::CONTROLLERS, print::kprintln, sync::MutexLike, thread::yield_thread,
    };

    let controller = {
        let controllers = CONTROLLERS.lock();
        assert!(
            !controllers.is_empty(),
            "no xHCI controllers were registered during discovery"
        );
        kprintln!("Found {} xHCI controller(s)", controllers.len());
        controllers[0].clone()
    };

    const POLL_TIME: u32 = 600_000;
    let mut polls = 0_u32;
    loop {
        if controller.devices.lock().len() >= 2 {
            break;
        }
        polls += 1;
        if polls >= POLL_TIME {
            panic!(
                "xHCI enumeration did not produce 2 devices in time (saw {})",
                controller.devices.lock().len()
            );
        }
        yield_thread();
    }

    let (devices_count, has_kbd, has_mouse) = {
        let devices = controller.devices.lock();
        let mut has_kbd = false;
        let mut has_mouse = false;
        for dev in devices.iter() {
            let Some(cfg) = dev.config.as_ref() else {
                continue;
            };
            for (iface, _eps) in &cfg.interfaces {
                let triple = (
                    iface.b_interface_class,
                    iface.b_interface_subclass,
                    iface.b_interface_protocol,
                );
                match triple {
                    (0x03, 0x01, 0x01) => has_kbd = true,
                    (0x03, 0x01, 0x02) => has_mouse = true,
                    _ => {}
                }
            }
        }
        (devices.len(), has_kbd, has_mouse)
    };

    kprintln!("Controller 0: {} USB device(s) enumerated", devices_count);
    assert!(has_kbd, "expected a HID boot keyboard interface");
    kprintln!("Found USB HID keyboard");
    assert!(has_mouse, "expected a HID boot mouse interface");
    kprintln!("Found USB HID mouse");

    kprintln!("USB xHCI integration test passed");
});
