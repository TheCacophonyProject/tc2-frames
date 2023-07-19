use font_kit::font::Font;
use minifb::{Key, ScaleMode, Window, WindowOptions};
use raqote::{DrawOptions, DrawTarget, Image, Point, SolidSource, Source};
use std::cell::RefCell;
use std::io::Read;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener};
use std::sync::mpsc::channel;
use std::sync::Mutex;
use std::thread;
use std::thread::sleep;
use std::time::Duration;

use font_kit::family_name::FamilyName;
use font_kit::properties::Properties;
use font_kit::source::SystemSource;
use local_ip_address::local_ip;
use mdns_sd::{ServiceDaemon, ServiceInfo};
use std::collections::HashMap;
use std::net::IpAddr::V4;

const WINDOW_WIDTH: usize = 640;
const WINDOW_HEIGHT: usize = 480;
const SEGMENT_LENGTH: usize = 9760;
const TELEMETRY_LENGTH: usize = 640;
type Frame = [u8; 38400];
pub static FRAME_BUFFER: DoubleBuffer = DoubleBuffer {
    front: Mutex::new(RefCell::new([0u8; 38400])),
    back: Mutex::new(RefCell::new([0u8; 38400])),
};
//
pub struct DoubleBuffer {
    pub front: Mutex<RefCell<Frame>>,
    pub back: Mutex<RefCell<Frame>>,
}

impl DoubleBuffer {
    pub fn swap(&self) {
        if let Ok(front) = self.front.lock() {
            if let Ok(back) = self.back.lock() {
                front.swap(&*back);
            }
        }
    }
}

pub unsafe fn u8_as_u16_slice(p: &[u8]) -> &[u16] {
    core::slice::from_raw_parts((p as *const [u8]) as *const u16, p.len() / 2)
}

fn img_from_buffer(image_data: &mut [u32; 160 * 120]) -> Image {
    let gradient = colorous::VIRIDIS;
    FRAME_BUFFER.swap();
    let fb = {
        let fb = FRAME_BUFFER.front.lock().unwrap();
        let x = fb.borrow();
        x.clone()
    };
    let fbu16: Vec<u16> = unsafe { u8_as_u16_slice(&fb) }[0..160 * 120]
        .iter()
        .map(|x| x.to_be())
        .collect();
    let max = fbu16.iter().filter(|&x| *x > 0).max().unwrap_or(&u16::MAX);
    let min = fbu16.iter().filter(|&x| *x > 0).min().unwrap_or(&u16::MIN);
    let range = max - min;
    let range = range.max(1);
    for y in 0..120 {
        for x in 0..160 {
            let val = fbu16[(y * 160 + x) as usize];
            //let val = (((val.wrapping_sub(*min)) as f32 / range as f32) * 255.0) as u32;
            let index = (y * 160 + x) as usize;
            let val = gradient.eval_continuous((val.wrapping_sub(*min)) as f64 / range as f64);
            image_data[index] =
                255 << 24 | (val.r as u32) << 16 | (val.g as u32) << 8 | (val.b as u32);
        }
    }
    let image = Image {
        width: 160,
        height: 120,
        data: &image_data[..],
    };
    image
}

// TODO: Frame telemetry - print the frame number on the frame, so we can see if we're missing any at this end.

// TODO: Motion detection:
// Stick frame telemetry at the start of the frame blob, and correlate the focus metric.

// 1. Look through all the videos we have, extract just the track regions, and work out what is the
// minimum temperature which is above all of the background (maybe with some minimum clustering criteria).

// Is this minimum temperature over the background reasonably linear?

// Output an averaged view of the previous frames?

// TODO: fps counter
// TODO: Send frame in 4 pieces, and do motion detection on each of them separately.
fn draw(dt: &mut DrawTarget, image_data: &mut [u32; 160 * 120], font: &Font) {
    let image = img_from_buffer(image_data);
    let scale_x = 1.0 / (WINDOW_WIDTH as f32 / 160.0);
    let scale_y = 1.0 / (WINDOW_HEIGHT as f32 / 120.0);
    let img = Source::Image(
        image,
        raqote::ExtendMode::Pad,
        raqote::FilterMode::Bilinear,
        raqote::Transform::scale(scale_x, scale_y),
    );
    dt.clear(SolidSource {
        r: 0x00,
        g: 0x00,
        b: 0x00,
        a: 0x00,
    });
    dt.fill_rect(
        0.0,
        0.0,
        WINDOW_WIDTH as f32,
        WINDOW_HEIGHT as f32,
        &img,
        &DrawOptions::new(),
    );

    // dt.draw_text(
    //     &font,*
    //     14.,
    //     &"Test string",
    //     Point::new(10., 490. + 14.),
    //     &Source::Solid(SolidSource::from_unpremultiplied_argb(
    //         0xff, 0xff, 0xff, 0xff,
    //     )),
    //     &DrawOptions::new(),
    // );
}

fn main() -> std::io::Result<()> {
    let my_local_ip = local_ip().unwrap();
    let port: u16 = 34254;

    match my_local_ip {
        V4(local_ip_v4) => {
            // Create a daemon
            let mdns = ServiceDaemon::new().expect("Failed to create daemon");

            // Create a service info.
            let service_type = "_mdns-tc2-frames._udp.local.";
            let instance_name = "tc2-frames";
            let port = 5200;
            let properties = [("property_1", "test"), ("property_2", "1234")];
            let my_service = ServiceInfo::new(
                service_type,
                instance_name,
                &format!("{}.local.", local_ip_v4),
                local_ip_v4,
                port,
                &properties[..],
            )
            .unwrap();

            // Register with the daemon, which publishes the service.
            mdns.register(my_service)
                .expect("Failed to register our service");
        }
        _ => panic!("Unexpected ipV6 address"),
    }

    let (tx, rx) = channel();
    let _ = thread::spawn(move || {
        match TcpListener::bind(SocketAddr::from((my_local_ip, port))) {
            Ok(listener) => {
                // accept connections and process them serially
                println!("Listening at {}", listener.local_addr().unwrap());
                for stream in listener.incoming() {
                    if let Ok(mut stream) = stream {
                        println!("Connection from {}", stream.peer_addr().unwrap());

                        let mut buffer = [0u8; 39040];
                        // ...
                        while let Ok(_) = stream.read_exact(&mut buffer) {
                            {
                                //println!("Got frame");
                                let fb = FRAME_BUFFER.back.lock().unwrap();
                                fb.borrow_mut().copy_from_slice(&buffer[TELEMETRY_LENGTH..]);
                            }
                            tx.send(1).unwrap();
                        }
                    }
                }
            }
            Err(err) => panic!("Error {:?}", err),
        }
    });

    let width = WINDOW_WIDTH;
    let height = WINDOW_HEIGHT;

    let mut draw_target = DrawTarget::new(width as i32, height as i32);
    let mut image_data = [0u32; 160 * 120];
    let mut window_options = WindowOptions::default();
    window_options.resize = true;
    window_options.scale_mode = ScaleMode::AspectRatioStretch;
    let mut window = Window::new("tc2-frames", width, height, window_options).unwrap_or_else(|e| {
        panic!("{}", e);
    });
    let font = SystemSource::new()
        .select_best_match(&[FamilyName::SansSerif], &Properties::new())
        .unwrap()
        .load()
        .unwrap();
    window
        .update_with_buffer(&draw_target.get_data(), width, height)
        .unwrap();
    while window.is_open() && !window.is_key_down(Key::Escape) {
        if let Ok(_) = rx.try_recv() {
            draw(&mut draw_target, &mut image_data, &font);
        } else {
            sleep(Duration::from_millis(16));
        }
        // We unwrap here as we want this code to exit if it fails. Real applications may want to handle this in a different way
        window
            .update_with_buffer(&draw_target.get_data(), width, height)
            .unwrap();
    }
    Ok(())
}
