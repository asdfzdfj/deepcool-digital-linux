use std::{
    fs::read_to_string,
    process::exit,
    thread::sleep,
    time::Duration
};
use cpu_monitor::CpuInstant;
use libc::geteuid;
use hidapi::{HidApi, HidDevice};
use clap::Parser;


const VENDOR: u16 = 0x3633;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Change temperature unit to Fahrenheit
    #[arg(short, long)]
    fahrenheit: bool,

    /// Change the polling rate in milliseconds
    #[arg(short, long, default_value_t = 750)]
    poll: u64,
}

fn main() {
    // Check root
    unsafe {
        if geteuid() != 0 {
            println!("Try to run the program as root!");
            exit(1);
        }
    }

    // Read args
    let args = Args::parse();

    // Find device
    let api = HidApi::new().expect("Failed to initialize HID API");
    let mut product_id = 0;
    for device in api.device_list() {
        if device.vendor_id() == VENDOR {
            product_id = device.product_id();
            println!("Device found: {}", device.product_string().unwrap());
            println!("Debug info: {:?}", device);
            break;
        }
    }
    if product_id == 0 {
        println!("Device not found!");
        exit(1);
    }
    
    // Connect
    let device = api.open(VENDOR, product_id).expect("Failed to open HID device");

    // Find CPU temp. sensor
    let cpu_hwmon_path = find_cpu_sensor();

    // Data block
    let mut data: [u8; 64] = [0; 64];
    data[0] = 16;
    data[1] = 104;
    data[2] = 1;
    data[3] = 1;
    
    // Init sequence
    println!("\nInit sequence:");
    {
        let mut init_data = data.clone();
        init_data[4] = 2;
        init_data[5] = 3;
        init_data[6] = 1;
        init_data[7] = 112;
        init_data[8] = 22;
        write_data(&device, &init_data);
        init_data[5] = 2;
        init_data[7] = 111;
        write_data(&device, &init_data);
    }

    // Display loop
    println!("\nSending status packets:");
    loop {
        // Initialize the packet
        let mut status_data = data.clone();
        status_data[4] = 11;
        status_data[5] = 1;
        status_data[6] = 2;
        status_data[7] = 5;

        // Read CPU utilization & power draw
        let cpu_util_start = CpuInstant::now().unwrap();
        let cpu_power_start = read_microjoules();

        // Wait
        sleep(Duration::from_millis(args.poll));

        // Finish reading
        let cpu_util_end = CpuInstant::now().unwrap();
        let cpu_power_end = read_microjoules();

        // ----- Write data to the package -----
        // Power Draw
        let cpu_power = (cpu_power_end - cpu_power_start) as f64 / (args.poll * 1000) as f64;
        let cpu_power_bytes = (cpu_power.round() as u16).to_be_bytes();
        status_data[8] = cpu_power_bytes[0];
        status_data[9] = cpu_power_bytes[1];

        // Temperature
        let temp = (get_temp(&cpu_hwmon_path, args.fahrenheit) as f32).to_be_bytes();
        status_data[10] = if args.fahrenheit {1} else {0};
        status_data[11] = temp[0];
        status_data[12] = temp[1];
        status_data[13] = temp[2];
        status_data[14] = temp[3];

        // Utilization
        let cpu_util = (cpu_util_end - cpu_util_start).non_idle() * 100.0;
        status_data[15] = (cpu_util).round() as u8;
        
        // Checksum & termination byte
        let checksum: u16 = status_data[1..=15].iter().map(|&x| x as u16).sum();
        status_data[16] = (checksum % 256) as u8;
        status_data[17] = 22;


        write_data(&device, &status_data);
    }       
}

// ------------------------- Functions -------------------------

/// I separated the writing so the main() is more readable.
fn write_data(device: &HidDevice, data: &[u8; 64]) {
    println!("Writing: {:?}", &data[0..=17]);
    device.write(data).expect("Failed to write data");
}

/// Looks for the appropriate CPU temperature sensor datastream in the hwmon folder.
pub fn find_cpu_sensor() -> String {
    let mut i = 0;
    loop {
        match read_to_string(format!("/sys/class/hwmon/hwmon{i}/name")) {
            Ok(data) => {
                let hwname = data.trim_end();
                if hwname == "k10temp" || hwname == "coretemp" {
                    return format!("/sys/class/hwmon/hwmon{i}/temp1_input");
                }
            },
            Err(_) => {
                println!("CPU temperature sensor not found!");
                exit(1);
            },
        }
        i += 1;
    }
}

/// Reads the value of the CPU temperature sensor and returns it as a rounded integer.
fn get_temp(cpu_sensor: &str, fahrenheit: bool) -> u8 {
    // Read sensor data
    let data = read_to_string(cpu_sensor).expect("Sensor data not found!");

    // Calculate temperature
    let mut k10temp = data.trim().parse::<u32>().unwrap();
    if fahrenheit {
        k10temp = k10temp * 9/5 + 32000
    }
    
    (k10temp as f32 / 1000 as f32).round() as u8
}


/// Reads the amount of energy used by the CPU and returns it as an unsigned integer. 
fn read_microjoules() -> u64 {
    let data = read_to_string("/sys/class/powercap/intel-rapl/intel-rapl:0/energy_uj")
        .expect("CPU power draw cannot be read!");
    
    data.trim().parse::<u64>().unwrap()
}
