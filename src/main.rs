use std::fs::{File, OpenOptions, read_to_string, write};
use std::os::unix::{fs::OpenOptionsExt, io::{RawFd, FromRawFd, IntoRawFd}};
use std::path::Path;
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime};
use std::{thread, time};
use input::{Libinput, LibinputInterface};
use libc::{O_RDONLY, O_RDWR, O_WRONLY};
use notify::{Watcher, RecursiveMode, RawEvent, raw_watcher};

struct Interface;

const CHECK_EVENTS_INTERVAL: u64 = 100;
const IDLE_MAX_TIME: u128 = 10000;
const BRIGHTNESS_OFF: char = '0';
const BRIGHTNESS_FILE: &str = "/sys/devices/platform/thinkpad_acpi/leds/tpacpi::kbd_backlight/brightness";
const BRIGHTNESS_HW_CHANGED_FILE: &str = "/sys/devices/platform/thinkpad_acpi/leds/tpacpi::kbd_backlight/brightness_hw_changed";

impl LibinputInterface for Interface {
    fn open_restricted(&mut self, path: &Path, flags: i32) -> Result<RawFd, i32> {
        OpenOptions::new()
            .custom_flags(flags)
            .read((flags & O_RDONLY != 0) | (flags & O_RDWR != 0))
            .write((flags & O_WRONLY != 0) | (flags & O_RDWR != 0))
            .open(path)
            .map(|file| file.into_raw_fd())
            .map_err(|err| err.raw_os_error().unwrap())
    }
    fn close_restricted(&mut self, fd: RawFd) {
        unsafe {
            File::from_raw_fd(fd);
        }
    }
}

fn read_brightness() -> char {
    return read_to_string(BRIGHTNESS_HW_CHANGED_FILE).unwrap().chars().nth(0).unwrap();
}

fn save_brightness(brightness_mode: char) {
    write(BRIGHTNESS_FILE, brightness_mode.to_string()).expect("Unable to write to brightness file");
}

fn main() {
    let mut input = Libinput::new_with_udev(Interface);
    input.udev_assign_seat("seat0").unwrap();

    let initial_brightness = read_brightness();

    let hardware_brightness = Arc::new(Mutex::new(initial_brightness));
    let c_hardware_brightness = Arc::clone(&hardware_brightness);

    let effective_brightness = Arc::new(Mutex::new(initial_brightness));
    let c_effective_brightness = Arc::clone(&effective_brightness);

    let timeout = Arc::new(Mutex::new(SystemTime::now()));
    let c_timeout = Arc::clone(&timeout);

    thread::spawn(move || {
        let (tx, rx) = channel();
        let mut watcher = raw_watcher(tx).unwrap();
        watcher.watch(BRIGHTNESS_HW_CHANGED_FILE, RecursiveMode::Recursive).unwrap();

        loop {
            match rx.recv() {
                Ok(RawEvent{path: Some(_path), op: Ok(_op), cookie: _}) => {
                    let current_brightness = read_brightness();
                    *c_hardware_brightness.lock().unwrap() = current_brightness;
                    *c_effective_brightness.lock().unwrap() = current_brightness;

                    if current_brightness != BRIGHTNESS_OFF {
                        *c_timeout.lock().unwrap() = SystemTime::now();
                    }
                },
                Ok(event) => println!("broken event: {:?}", event),
                Err(e) => println!("watch error: {:?}", e),
            }
        }
    });

    loop {
        thread::sleep(time::Duration::from_millis(CHECK_EVENTS_INTERVAL));
        input.dispatch().unwrap();

        for _event in &mut input {
            *timeout.lock().unwrap() = SystemTime::now();

            if *hardware_brightness.lock().unwrap() == BRIGHTNESS_OFF && *effective_brightness.lock().unwrap() != BRIGHTNESS_OFF {
                *hardware_brightness.lock().unwrap() = *effective_brightness.lock().unwrap();
                save_brightness(*effective_brightness.lock().unwrap());
                break;
            }
        }

        let timeout_ellapsed = SystemTime::now()
            .duration_since(*timeout.lock().unwrap())
            .expect("time is running backwards");

        if timeout_ellapsed.as_millis() > IDLE_MAX_TIME && *hardware_brightness.lock().unwrap() != BRIGHTNESS_OFF {
            *hardware_brightness.lock().unwrap() = BRIGHTNESS_OFF;
            save_brightness(BRIGHTNESS_OFF);
        }
    }
}
