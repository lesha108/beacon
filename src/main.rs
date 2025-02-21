use clap::{ arg, value_parser, ArgAction, Command };
use industrial_io as iio;
use std::{ any::TypeId, process, thread, time, sync::{ atomic::{ AtomicBool, Ordering }, Arc } };

use morse_codec::{ encoder::{ Encoder, MorseCharray, SDM }, Character };
use morse_codec::encoder::SDM::*;

// позывной для передачи маяком
const DEFAULT_CALLSIGN: &str = "RAEM";

// настройки передатчика
const BANDWIDTH: i64 = 2_000_000;
const SAMPLING_FREQ: i64 = 2_500_000;
const RF_FREQ: i64 = 144_400_000;

fn main() {
    let args = Command::new("beacon")
        .version(clap::crate_version!())
        .about("LibreSDR RF beacon. (C) R2AJP")
        .disable_help_flag(true)
        .disable_version_flag(true)
        .args(
            &[
                arg!(-h --host "Use the network backend with the specified host").action(
                    ArgAction::Set
                ),
                arg!(-u --uri "Use the context with the provided URI")
                    .action(ArgAction::Set)
                    .conflicts_with("host"),
                arg!(-f --frequency "Specifies the TX frequency")
                    .action(ArgAction::Set)
                    .value_parser(value_parser!(i64)),
                arg!(-c --callsign "Specifies the callsign to transmit")
                    .action(ArgAction::Set)
                    .default_value(DEFAULT_CALLSIGN),
                arg!(-b --beaconoff "Disables CW modulation").action(ArgAction::SetTrue),
                arg!(-'v' --version "Print version information").action(ArgAction::Version),
                arg!(-'?' --help "Print help information").global(true).action(ArgAction::Help),
            ]
        )
        .get_matches();

    let freq = *args.get_one("frequency").unwrap_or(&RF_FREQ);
    if freq > 6_000_000_000 || freq < 70_000_000 {
        println!("Freq out of range 70MHz..6HHz! Exiting");
        process::exit(1);
    }
    let callsign_str = args.get_one::<String>("callsign").unwrap();
    if callsign_str.len() > 12 {
        println!("Callsign too long! Exiting");
        process::exit(1);
    }

    // true - off CW mod
    let beacon_mode_off: bool = *args.get_one("beaconoff").unwrap();
    //print!("beaconmode: {beacon_mode_off}");

    let ctx = (
        if let Some(host) = args.get_one::<String>("host") {
            println!("Using host: {}", host);
            iio::Context::with_backend(iio::Backend::Network(host))
        } else if let Some(uri) = args.get_one::<String>("uri") {
            println!("Using URI: {}", uri);
            iio::Context::from_uri(uri)
        } else {
            iio::Context::new()
        }
    ).unwrap_or_else(|_err| {
        println!("Couldn't open IIO context.");
        process::exit(1);
    });

    let mut trigs = Vec::new();

    if ctx.num_devices() == 0 {
        println!("No devices in the default IIO context");
        process::exit(1);
    } else {
        println!("IIO Devices:");
        for dev in ctx.devices() {
            if dev.is_trigger() {
                if let Some(id) = dev.id() {
                    trigs.push(id);
                }
            } else {
                print!("  {} ", dev.id().unwrap_or_default());
                print!("[{}]", dev.name().unwrap_or_default());
                println!(": {} channel(s)", dev.num_channels());
            }
        }

        if !trigs.is_empty() {
            println!("\nTriggers Devices:");
            for s in trigs {
                println!("  {}", s);
            }
        }
    }

    // здесь кодирование сигнала и настройка тайминга для передачи морзе
    const MESSAGE_MAX_LENGTH: usize = 64;
    let mut encoder = Encoder::<MESSAGE_MAX_LENGTH>::new().with_message(callsign_str, true).build();

    if beacon_mode_off == false {
        println!("Callsign string is: {}", encoder.message.as_str());
        //println!("Message length: {}", encoder.message.len());
        println!("Morse encoded version:");
        println!();

        encoder.encode_message_all();
        let encoded_charrays = encoder.get_encoded_message_as_morse_charrays();

        encoded_charrays.for_each(|charray| {
            print_morse_charray(charray.unwrap());
        });
        println!();
    }

    /*println!("Morse encoding signal duration multipliers:");
    let encoded_sdms = encoder.get_encoded_message_as_sdm_arrays();
    encoded_sdms.for_each(|sdm| {
        println!("{:?}", sdm);
    });*/

    // devices config

    let phy = if let Some(found) = ctx.find_device("ad9361-phy") {
        found
    } else {
        println!("No ad9361-phy found in the IIO context");
        process::exit(1);
    };

    // Acquiring AD9361 streaming devices
    let rx_dev = if let Some(found) = ctx.find_device("cf-ad9361-lpc") {
        found
    } else {
        println!("No cf-ad9361-lpc found in the IIO context");
        process::exit(1);
    };
    let tx_dev = if let Some(found) = ctx.find_device("cf-ad9361-dds-core-lpc") {
        found
    } else {
        println!("No cf-ad9361-dds-core-lpc found in the IIO context");
        process::exit(1);
    };

    //  finds AD9361 phy IIO configuration 0 channel
    let rx_channel = if let Some(found) = phy.find_input_channel("voltage0") {
        found
    } else {
        println!("No phy rx voltage0 found in the IIO context");
        process::exit(1);
    };
    let tx_channel = if let Some(found) = phy.find_output_channel("voltage0") {
        found
    } else {
        println!("No phy tx voltage0 found in the IIO context");
        process::exit(1);
    };

    // finds AD9361 local oscillator IIO configuration channels
    let rx_lo_channel = if let Some(found) = phy.find_output_channel("altvoltage0") {
        found
    } else {
        println!("No phy rxlo altvoltage0 found in the IIO context");
        process::exit(1);
    };
    let tx_lo_channel = if let Some(found) = phy.find_output_channel("altvoltage1") {
        found
    } else {
        println!("No phy txlo altvoltage1 found in the IIO context");
        process::exit(1);
    };

    //  finds attr bandwith & freq
    let rx_channel_rf_bandwidth = if let Some(found) = rx_channel.find_attr("rf_bandwidth") {
        found
    } else {
        println!("No rx rf_bandwidth found in the IIO context");
        process::exit(1);
    };
    let rx_channel_sampling_frequency = if
        let Some(found) = rx_channel.find_attr("sampling_frequency")
    {
        found
    } else {
        println!("No rx sampling_frequency found in the IIO context");
        process::exit(1);
    };
    let tx_channel_rf_bandwidth = if let Some(found) = tx_channel.find_attr("rf_bandwidth") {
        found
    } else {
        println!("No tx rf_bandwidth found in the IIO context");
        process::exit(1);
    };
    let tx_channel_sampling_frequency = if
        let Some(found) = tx_channel.find_attr("sampling_frequency")
    {
        found
    } else {
        println!("No tx sampling_frequency found in the IIO context");
        process::exit(1);
    };
    let tx_channel_hardwaregain = if let Some(found) = tx_channel.find_attr("hardwaregain") {
        found
    } else {
        println!("No tx hardwaregain found in the IIO context");
        process::exit(1);
    };
    let tx_channel_powerdown = if let Some(found) = tx_lo_channel.find_attr("powerdown") {
        found
    } else {
        println!("No tx powerdown found in the IIO context");
        process::exit(1);
    };

    // установка полосы пропускания и частоты дискретизации RX/TX
    if let Err(_) = rx_channel.attr_write_int(&rx_channel_rf_bandwidth, BANDWIDTH) {
        println!("Err rx rf_bandwidth write");
        process::exit(1);
    }
    if let Err(_) = rx_channel.attr_write_int(&rx_channel_sampling_frequency, SAMPLING_FREQ) {
        println!("Err rx sampling_frequency write");
        process::exit(1);
    }
    if let Err(_) = tx_channel.attr_write_int(&tx_channel_rf_bandwidth, BANDWIDTH) {
        println!("Err tx rf_bandwidth write");
        process::exit(1);
    }
    if let Err(_) = tx_channel.attr_write_int(&tx_channel_sampling_frequency, SAMPLING_FREQ) {
        println!("Err tx sampling_frequency write");
        process::exit(1);
    }

    // установка частоты RX/TX
    let rx_lo_channel_frequency = if let Some(found) = rx_lo_channel.find_attr("frequency") {
        found
    } else {
        println!("No rx_lo frequency found in the IIO context");
        process::exit(1);
    };
    let tx_lo_channel_frequency = if let Some(found) = tx_lo_channel.find_attr("frequency") {
        found
    } else {
        println!("No tx_lo frequency found in the IIO context");
        process::exit(1);
    };
    if let Err(_) = rx_lo_channel.attr_write_int(&rx_lo_channel_frequency, freq) {
        println!("Err rx_lo_channel_frequency write");
        process::exit(1);
    }
    if let Err(_) = tx_lo_channel.attr_write_int(&tx_lo_channel_frequency, freq) {
        println!("Err tx_lo_channel_frequency write");
        process::exit(1);
    }

    // поиск каналов I/Q
    let rx_channel_i = if let Some(found) = rx_dev.find_input_channel("voltage0") {
        found
    } else {
        println!("No rx_dev rx voltage0 found in the IIO context");
        process::exit(1);
    };
    let rx_channel_q = if let Some(found) = rx_dev.find_input_channel("voltage1") {
        found
    } else {
        println!("No rx_dev rx voltage1 found in the IIO context");
        process::exit(1);
    };
    let tx_channel_i = if let Some(found) = tx_dev.find_output_channel("voltage0") {
        found
    } else {
        println!("No tx_dev tx voltage0 found in the IIO context");
        process::exit(1);
    };
    let tx_channel_q = if let Some(found) = tx_dev.find_output_channel("voltage1") {
        found
    } else {
        println!("No tx_dev tx voltage1 found in the IIO context");
        process::exit(1);
    };

    // разрешение работы каналов I/Q
    let mut nchan = 0;
    for chan in rx_dev.channels() {
        if
            chan.type_of() == Some(TypeId::of::<i16>()) &&
            (chan.id() == Some("voltage0".to_string()) || chan.id() == Some("voltage1".to_string()))
        {
            nchan += 1;
            chan.enable();
        }
    }
    if nchan == 0 {
        println!("Couldn't find any signed 16-bit channels to capture.");
        process::exit(1);
    }
    println!("RX channels enabled {nchan}.");

    nchan = 0;
    for chan in tx_dev.channels() {
        if
            chan.type_of() == Some(TypeId::of::<i16>()) &&
            (chan.id() == Some("voltage0".to_string()) || chan.id() == Some("voltage1".to_string()))
        {
            nchan += 1;
            chan.enable();
        }
    }
    if nchan == 0 {
        println!("Couldn't find any signed 16-bit channels to transmit.");
        process::exit(1);
    }
    println!("TX channels enabled {nchan}.");

    // Creating non-cyclic IIO buffers with 1 MiS
    let mut rx_buf = rx_dev.create_buffer(1_000_000, false).unwrap_or_else(|err| {
        println!("Unable to create rx buffer: {}", err);
        process::exit(1);
    });
    let mut tx_buf = tx_dev.create_buffer(1_000_000, false).unwrap_or_else(|err| {
        println!("Unable to create tx buffer: {}", err);
        process::exit(1);
    });

    // записываем семплы в буфер передачи. Будет просто несущая на частоте LO
    for txchan_i in tx_buf.channel_iter_mut::<i16>(&tx_channel_i) {
        *txchan_i = 0x7ff0;
        //*txchan_i = 0;
    }
    for txchan_q in tx_buf.channel_iter_mut::<i16>(&tx_channel_q) {
        *txchan_q = 0;
    }

    // ---- Handle ^C since we want a graceful shutdown -----

    let quit = Arc::new(AtomicBool::new(false));
    let q = quit.clone();

    ctrlc
        ::set_handler(move || {
            q.store(true, Ordering::SeqCst);
        })
        .expect("Error setting Ctrl-C handler");

    println!("Start transmitting on freq {freq}...");
    println!("Press Ctrl-C to stop");
    // powering up transmitter
    if let Err(_) = tx_lo_channel.attr_write_int(&tx_channel_powerdown, 0) {
        println!("Err powerup");
        process::exit(1);
    }
    // attenuation tx
    if let Err(_) = tx_channel.attr_write_int(&tx_channel_hardwaregain, -10) {
        println!("Err set hwgain");
        process::exit(1);
    }

    // один раз закинуть буфер по какой-то причине достаточно Ж-/
    // программа считает его циклическим буфером?
    if let Err(_) = tx_buf.push() {
        println!("Error buffer push");
        process::exit(1);
    }

    if beacon_mode_off == true {
        // передаем в цикле просто несущую до нажатия Ctrl-C
        while !quit.load(Ordering::SeqCst) {
            thread::sleep(time::Duration::from_millis(20));
            /*            if let Err(_) = tx_buf.push() {
                println!("Error buffer push");
                process::exit(1);
            }
  */
        }
    } else {
        // передаем в цикле позывной маяка до нажатия Ctrl-C
        while !quit.load(Ordering::SeqCst) {
            encoder.get_encoded_message_as_sdm_arrays().for_each(|sdm| {
                match sdm {
                    Some(symbol) => {
                        for smb in symbol {
                            match smb {
                                High(t1) => {
                                    if
                                        let Err(_) = tx_channel.attr_write_int(
                                            &tx_channel_hardwaregain,
                                            -10
                                        )
                                    {
                                        println!("Err set hwgain");
                                        process::exit(1);
                                    }
                                    thread::sleep(time::Duration::from_millis((50 * t1).into()));
                                }
                                Low(t2) => {
                                    if
                                        let Err(_) = tx_channel.attr_write_int(
                                            &tx_channel_hardwaregain,
                                            -40
                                        )
                                    {
                                        println!("Err set hwgain");
                                        process::exit(1);
                                    }
                                    thread::sleep(time::Duration::from_millis((50 * t2).into()));
                                }
                                Empty => {}
                            }
                        }
                    }
                    None => {}
                }

                //println!("{:?}", sdm);
            });
            thread::sleep(time::Duration::from_millis(900));
        }
    }

    // powering down transmitter
    if let Err(_) = tx_lo_channel.attr_write_int(&tx_channel_powerdown, 1) {
        println!("Err powerdown");
        process::exit(1);
    }
    println!("All was OK! Bye");
}

fn print_morse_charray(mchar: MorseCharray) {
    for ch in mchar.iter().filter(|ch| ch.is_some()) {
        print!("{}", ch.unwrap() as char);
    }
    print!(" ");
}
