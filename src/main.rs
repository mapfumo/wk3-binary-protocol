#![no_std]
#![no_main]

use panic_probe as _;
use defmt_rtt as _;

#[rtic::app(device = stm32f4xx_hal::pac, peripherals = true)]
mod app {
    use stm32f4xx_hal::{
        prelude::*,
        gpio::{Output, Pin},
        pac,
        timer::{CounterHz, Event, Delay},
        serial::{Serial, Config as SerialConfig, Event as SerialEvent},
        i2c::I2c,
        rcc::Config,
    };

    use shared_bus::CortexMMutex;
    use ssd1306::{prelude::*, Ssd1306, mode::BufferedGraphicsMode};
    use display_interface_i2c::I2CInterface;
    use embedded_graphics::{
        mono_font::{ascii::FONT_6X10, MonoTextStyleBuilder},
        pixelcolor::BinaryColor,
        prelude::*,
        text::Text,
    };
    use heapless::{String, Vec};
    use core::fmt::Write as _;

    use sht3x::{SHT3x, Repeatability, Address as ShtAddress};
    use bme680::{Bme680, I2CAddress, IIRFilterSize, OversamplingSetting, SettingsBuilder, PowerMode};
    use core::time::Duration;

    // --- Configuration Constants ---
    const NODE_ID: &str = "N1";              // Node identifier for display
    const AUTO_TX_INTERVAL_SECS: u32 = 10;  // Auto-transmit every 10 seconds
    const NETWORK_ID: u8 = 18;               // LoRa network ID
    const LORA_FREQ: u32 = 915;              // LoRa frequency in MHz (915 for US)

    // --- Binary Protocol Data Structures ---
    use serde::{Serialize, Deserialize};

    /// Sensor data packet for binary transmission
    /// Size: ~12 bytes (postcard serialized) vs 24 bytes (text format)
    #[derive(Debug, Clone, Copy, Serialize, Deserialize)]
    pub struct SensorDataPacket {
        pub seq_num: u16,           // Sequence number for duplicate detection
        pub temperature: i16,       // Temperature in centidegrees (e.g., 2710 = 27.1°C)
        pub humidity: u16,          // Humidity in basis points (e.g., 5600 = 56.0%)
        pub gas_resistance: u32,    // Gas resistance in ohms
    }

    /// ACK/NACK packet for acknowledgment
    /// Size: 3 bytes (1 byte msg_type + 2 bytes seq_num)
    #[derive(Debug, Clone, Copy, Serialize, Deserialize)]
    pub struct AckPacket {
        pub msg_type: u8,   // 1 = ACK (success), 2 = NACK (CRC failure)
        pub seq_num: u16,   // Which packet we're acknowledging
    }

    // Message type constants
    const MSG_TYPE_ACK: u8 = 1;
    const MSG_TYPE_NACK: u8 = 2;

    // Transmission retry configuration
    const MAX_RETRIES: u8 = 3;
    const ACK_TIMEOUT_SECS: u32 = 2;  // Wait 2 seconds for ACK before retry

    /// Transmission state for reliable delivery
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub enum TxState {
        Idle,                    // Waiting for next transmission trigger
        WaitingForAck {          // Packet sent, waiting for ACK
            seq_num: u16,        // Which packet we're waiting for
            timeout_counter: u32, // Countdown in seconds until timeout
            retry_count: u8,     // How many retries attempted so far
        },
    }

    /// Calculate CRC-16 checksum for data integrity
    /// Uses CRC-16-IBM-3740 (CCITT with 0xFFFF initial value)
    fn calculate_crc16(data: &[u8]) -> u16 {
        use crc::{Crc, CRC_16_IBM_3740};
        const CRC16: Crc<u16> = Crc::<u16>::new(&CRC_16_IBM_3740);
        CRC16.checksum(data)
    }

    /// Parse ACK/NACK message from Node 2
    /// Format: +RCV=<Address>,<Length>,<BinaryData>,<RSSI>,<SNR>\r\n
    fn parse_ack_message(buffer: &[u8]) -> Option<AckPacket> {
        // Check prefix: must start with "+RCV="
        if buffer.len() < 10 || &buffer[0..5] != b"+RCV=" {
            return None;
        }

        // Find first two commas
        let mut comma1_pos = None;
        let mut comma2_pos = None;

        for (i, &byte) in buffer[5..].iter().enumerate() {
            if byte == b',' {
                if comma1_pos.is_none() {
                    comma1_pos = Some(5 + i);
                } else if comma2_pos.is_none() {
                    comma2_pos = Some(5 + i);
                    break;
                }
            }
        }

        let comma1 = comma1_pos?;
        let comma2 = comma2_pos?;

        // Extract length
        let len_bytes = &buffer[comma1 + 1..comma2];
        let len_str = core::str::from_utf8(len_bytes).ok()?;
        let payload_len: usize = len_str.parse().ok()?;

        // Extract binary payload
        let payload_start = comma2 + 1;
        let payload_end = payload_start + payload_len;

        if payload_end > buffer.len() {
            return None;
        }

        let binary_payload = &buffer[payload_start..payload_end];

        // Deserialize ACK packet (no CRC on ACK packets - they're tiny!)
        postcard::from_bytes(binary_payload).ok()
    }

    // --- Bridge for embedded-hal 1.0 -> 0.2.7 ---
    pub struct I2cCompat<I2C>(pub I2C);

    impl<I2C> embedded_hal_0_2::blocking::i2c::Write for I2cCompat<I2C>
    where I2C: embedded_hal::i2c::I2c {
        type Error = I2C::Error;
        fn write(&mut self, addr: u8, bytes: &[u8]) -> Result<(), Self::Error> {
            self.0.write(addr, bytes)
        }
    }

    impl<I2C> embedded_hal_0_2::blocking::i2c::Read for I2cCompat<I2C>
    where I2C: embedded_hal::i2c::I2c {
        type Error = I2C::Error;
        fn read(&mut self, addr: u8, buffer: &mut [u8]) -> Result<(), Self::Error> {
            self.0.read(addr, buffer)
        }
    }

    impl<I2C> embedded_hal_0_2::blocking::i2c::WriteRead for I2cCompat<I2C>
    where I2C: embedded_hal::i2c::I2c {
        type Error = I2C::Error;
        fn write_read(&mut self, addr: u8, bytes: &[u8], buffer: &mut [u8]) -> Result<(), Self::Error> {
            self.0.write_read(addr, bytes, buffer)
        }
    }

    type MyI2c = I2c<pac::I2C1>;
    type ShtDelay = Delay<pac::TIM5, 1000000>;
    type BmeDelay = Delay<pac::TIM3, 1000000>;
    type BusManager = shared_bus::BusManager<CortexMMutex<I2cCompat<MyI2c>>>;
    type I2cProxy = shared_bus::I2cProxy<'static, CortexMMutex<I2cCompat<MyI2c>>>;
    
    type LoraDisplay = Ssd1306<I2CInterface<I2cProxy>, DisplaySize128x64, BufferedGraphicsMode<DisplaySize128x64>>;

    #[shared]
    struct Shared {
        lora_uart: Serial<pac::UART4>,
        display: LoraDisplay,
        sht31: SHT3x<I2cProxy, ShtDelay>,
        bme680: Bme680<I2cProxy, BmeDelay>,
        tx_state: TxState,     // Transmission state machine (shared between tim2 and uart4)
    }

    #[local]
    struct Local {
        led: Pin<'A', 5, Output>,
        button: Pin<'C', 13>,  // Blue button on Nucleo (PC13)
        timer: CounterHz<pac::TIM2>,
        bme_delay: BmeDelay,
        packet_counter: u32,   // Counts packets sent
        tx_countdown: u32,     // Seconds until next auto-transmit
        rx_buffer: Vec<u8, 128>,  // Buffer for incoming ACK/NACK packets
    }

    // Helper function to send AT command and wait for response
    fn send_at_command(uart: &mut Serial<pac::UART4>, cmd: &str) {
        defmt::info!("Sending AT command: {}", cmd);

        // Send command
        for byte in cmd.as_bytes() {
            let _ = nb::block!(uart.write(*byte));
        }

        // Send \r\n
        let _ = nb::block!(uart.write(b'\r'));
        let _ = nb::block!(uart.write(b'\n'));

        // Wait a bit for module to process
        cortex_m::asm::delay(8_400_000); // ~100ms at 84 MHz
    }

    #[init]
    fn init(cx: init::Context) -> (Shared, Local, init::Monotonics) {
        let dp = cx.device;
        
        // 1. Configure RCC clocks (0.23.0 API uses freeze with Config)
        let mut rcc = dp.RCC.freeze(Config::hsi().sysclk(84.MHz()));

        // 2. Split GPIOs (requires &mut rcc in 0.23.0)
        let gpioa = dp.GPIOA.split(&mut rcc);
        let gpiob = dp.GPIOB.split(&mut rcc);
        let gpioc = dp.GPIOC.split(&mut rcc);

        let led = gpioa.pa5.into_push_pull_output();
        let button = gpioc.pc13;  // Blue button (has built-in pull-up, active-low)

        // Create delay instances for SHT31 and BME680
        // SHT31 takes ownership of its delay (TIM5)
        let sht_delay = dp.TIM5.delay_us(&mut rcc);
        // BME680 delay (TIM3) will be moved to Local for use in handler
        let mut bme_delay = dp.TIM3.delay_us(&mut rcc);

        // --- UART4 ---
        let tx = gpioc.pc10.into_alternate();
        let rx = gpioc.pc11.into_alternate();
        let mut lora_uart = Serial::new(
            dp.UART4,
            (tx, rx),
            SerialConfig::default().baudrate(115200.bps()),
            &mut rcc
        ).unwrap();

        // Configure LoRa module before enabling RX interrupt
        defmt::info!("Configuring LoRa module (Node 1)...");
        send_at_command(&mut lora_uart, "AT");
        send_at_command(&mut lora_uart, "AT+ADDRESS=1");

        let mut cmd_buf: String<32> = String::new();
        let _ = core::write!(cmd_buf, "AT+NETWORKID={}", NETWORK_ID);
        send_at_command(&mut lora_uart, cmd_buf.as_str());

        cmd_buf.clear();
        let _ = core::write!(cmd_buf, "AT+BAND={}000000", LORA_FREQ);
        send_at_command(&mut lora_uart, cmd_buf.as_str());

        send_at_command(&mut lora_uart, "AT+PARAMETER=7,9,1,7");

        // Flush any pending responses from configuration
        while lora_uart.read().is_ok() {}

        // Explicitly clear any error flags (especially ORE) before enabling interrupt
        let uart_ptr = unsafe { &*pac::UART4::ptr() };
        let sr = uart_ptr.sr().read();
        if sr.ore().bit_is_set() || sr.nf().bit_is_set() || sr.fe().bit_is_set() {
            let _ = uart_ptr.dr().read();
            defmt::info!("N1 INIT: Cleared error flags (ORE={} NF={} FE={})",
                sr.ore().bit_is_set(), sr.nf().bit_is_set(), sr.fe().bit_is_set());
        }

        defmt::info!("LoRa module configured");

        lora_uart.listen(SerialEvent::RxNotEmpty);

        // --- I2C1 ---
        let scl = gpiob.pb8.into_alternate_open_drain();
        let sda = gpiob.pb9.into_alternate_open_drain();
        let i2c = I2c::new(dp.I2C1, (scl, sda), 100.kHz(), &mut rcc);
        
        let i2c_compat = I2cCompat(i2c);
        let bus: &'static BusManager = shared_bus::new_cortexm!(I2cCompat<MyI2c> = i2c_compat).unwrap();

        // --- Sensors ---
        let sht31 = SHT3x::new(bus.acquire_i2c(), sht_delay, ShtAddress::Low);
        let mut bme680 = Bme680::init(bus.acquire_i2c(), &mut bme_delay, I2CAddress::Secondary).unwrap();
        
        let settings = SettingsBuilder::new()
            .with_humidity_oversampling(OversamplingSetting::OS2x)
            .with_pressure_oversampling(OversamplingSetting::OS4x)
            .with_temperature_oversampling(OversamplingSetting::OS2x)
            .with_temperature_filter(IIRFilterSize::Size3)
            .with_gas_measurement(Duration::from_millis(150), 300, 25)
            .with_run_gas(true)
            .build();
        let _ = bme680.set_sensor_settings(&mut bme_delay, settings);

        // --- Display ---
        let interface = I2CInterface::new(bus.acquire_i2c(), 0x3C, 0x40);
        let mut display = Ssd1306::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
            .into_buffered_graphics_mode();
        display.init().unwrap();

        // --- Timer ---
        let mut timer = dp.TIM2.counter_hz(&mut rcc);
        timer.start(1.Hz()).unwrap();  // Still ticks at 1 Hz for countdown
        timer.listen(Event::Update);

        (
            Shared {
                lora_uart,
                display,
                sht31,
                bme680,
                tx_state: TxState::Idle,              // Start in Idle state
            },
            Local {
                led,
                button,
                timer,
                bme_delay,
                packet_counter: 0,                    // Start at packet #0
                tx_countdown: AUTO_TX_INTERVAL_SECS,  // First TX in 10 seconds
                rx_buffer: Vec::new(),                // Empty RX buffer
            },
            init::Monotonics()
        )
    }

    #[task(binds = TIM2, shared = [sht31, bme680, display, lora_uart, tx_state], local = [led, button, timer, bme_delay, packet_counter, tx_countdown])]
    fn tim2_handler(mut cx: tim2_handler::Context) {
        cx.local.timer.clear_flags(stm32f4xx_hal::timer::Flag::Update);
        cx.local.led.toggle();

        // State machine: Handle ACK timeout
        cx.shared.tx_state.lock(|state| {
            match *state {
                TxState::WaitingForAck { seq_num, timeout_counter, retry_count } => {
                    if timeout_counter > 0 {
                        // Countdown timeout
                        *state = TxState::WaitingForAck {
                            seq_num,
                            timeout_counter: timeout_counter - 1,
                            retry_count,
                        };
                    } else {
                        // Timeout reached - count it as a retry
                        let new_retry_count = retry_count + 1;
                        if new_retry_count < MAX_RETRIES {
                            defmt::warn!("ACK timeout for packet #{}, attempt {}/{}, will keep waiting",
                                seq_num, new_retry_count + 1, MAX_RETRIES);
                            // Keep waiting with incremented retry counter and reset timeout
                            *state = TxState::WaitingForAck {
                                seq_num,
                                timeout_counter: ACK_TIMEOUT_SECS,
                                retry_count: new_retry_count,
                            };
                        } else {
                            defmt::error!("Max retries ({}) exceeded for packet #{}, giving up", MAX_RETRIES, seq_num);
                            *state = TxState::Idle;
                        }
                    }
                }
                TxState::Idle => {
                    // Normal operation
                }
            }
        });

        // Determine if we should transmit this cycle
        let mut should_transmit = false;
        let mut trigger_source = "AUTO";

        // Check button (active-low: pressed = low)
        if cx.local.button.is_low() {
            defmt::info!("Button pressed - triggering immediate transmission");
            should_transmit = true;
            trigger_source = "BTN";
            *cx.local.tx_countdown = AUTO_TX_INTERVAL_SECS;  // Reset countdown
        } else {
            // Auto-transmit countdown
            if *cx.local.tx_countdown > 0 {
                *cx.local.tx_countdown -= 1;
            }

            if *cx.local.tx_countdown == 0 {
                defmt::info!("Auto-transmit countdown reached 0");
                should_transmit = true;
                *cx.local.tx_countdown = AUTO_TX_INTERVAL_SECS;  // Reset countdown
            }
        }

        // Only read sensors and transmit if triggered AND in Idle state
        let is_idle = cx.shared.tx_state.lock(|state| *state == TxState::Idle);
        if should_transmit && is_idle {
            let delay = cx.local.bme_delay;

            cx.shared.bme680.lock(|bme| {
                let _ = bme.set_sensor_mode(delay, PowerMode::ForcedMode);
            });

            delay.delay_ms(200u32);

            cx.shared.bme680.lock(|bme| {
                if let Ok((data, _state)) = bme.get_sensor_data(delay) {
                    // BME680 used only for gas resistance (SHT31 is more accurate for temp/humidity)
                    let gas = data.gas_resistance_ohm();

                    cx.shared.sht31.lock(|sht| {
                        if let Ok(meas) = sht.measure(Repeatability::High) {
                            let temp_c = meas.temperature as f32 / 100.0;
                            let humid_pct = meas.humidity as f32 / 100.0;

                            // Increment packet counter
                            *cx.local.packet_counter += 1;

                            cx.shared.display.lock(|disp: &mut LoraDisplay| {
                                let _ = disp.clear(BinaryColor::Off);
                                let style = MonoTextStyleBuilder::new()
                                    .font(&FONT_6X10)
                                    .text_color(BinaryColor::On)
                                    .build();

                                let mut buf: String<64> = String::new();
                                // Line 1: Temp & Humidity (compact)
                                let _ = core::write!(buf, "T:{:.1}C H:{:.0}%", temp_c, humid_pct);
                                Text::new(&buf, Point::new(0, 8), style).draw(disp).ok();

                                buf.clear();
                                // Line 2: Gas resistance
                                let _ = core::write!(buf, "Gas:{:.0}k", gas as f32 / 1000.0);
                                Text::new(&buf, Point::new(0, 20), style).draw(disp).ok();

                                buf.clear();
                                // Line 3: Node ID and TX status with packet counter
                                let _ = core::write!(buf, "{} TX:{} #{:04}", NODE_ID, trigger_source, *cx.local.packet_counter);
                                Text::new(&buf, Point::new(0, 32), style).draw(disp).ok();

                                buf.clear();
                                // Line 4: Network ID and frequency
                                let _ = core::write!(buf, "Net:{} {}MHz", NETWORK_ID, LORA_FREQ);
                                Text::new(&buf, Point::new(0, 44), style).draw(disp).ok();

                                buf.clear();
                                // Line 5: Countdown to next auto-TX
                                let _ = core::write!(buf, "Next:{}s", *cx.local.tx_countdown);
                                Text::new(&buf, Point::new(0, 56), style).draw(disp).ok();

                                let _ = disp.flush();
                            });

                            let current_seq = *cx.local.packet_counter as u16;
                            let mut tx_success = false;

                            cx.shared.lora_uart.lock(|uart| {
                                // === BINARY PROTOCOL ===
                                // Convert to centidegrees and basis points for binary protocol
                                let temp_centidegrees = (temp_c * 10.0) as i16;
                                let humid_basis_points = (humid_pct * 100.0) as u16;

                                let binary_packet = SensorDataPacket {
                                    seq_num: current_seq,
                                    temperature: temp_centidegrees,
                                    humidity: humid_basis_points,
                                    gas_resistance: gas,
                                };

                                // Serialize to binary
                                let mut binary_buffer = [0u8; 32];
                                match postcard::to_slice(&binary_packet, &mut binary_buffer) {
                                    Ok(serialized) => {
                                        let data_len = serialized.len();
                                        let crc = calculate_crc16(serialized);

                                        // Total payload: data + 2-byte CRC
                                        let total_len = data_len + 2;

                                        defmt::info!("Binary packet: {} bytes data + 2 bytes CRC = {} total, CRC: 0x{:04X}",
                                            data_len, total_len, crc);

                                        // Send AT command prefix: "AT+SEND=2,<total_length>,"
                                        let cmd_prefix = "AT+SEND=2,";
                                        for b in cmd_prefix.as_bytes() {
                                            let _ = nb::block!(uart.write(*b));
                                        }

                                        // Send total length as ASCII (includes CRC)
                                        let mut len_str: String<8> = String::new();
                                        let _ = core::write!(len_str, "{},", total_len);
                                        for b in len_str.as_bytes() {
                                            let _ = nb::block!(uart.write(*b));
                                        }

                                        // Send binary payload (data)
                                        for b in serialized {
                                            let _ = nb::block!(uart.write(*b));
                                        }

                                        // Send CRC-16 (big-endian: high byte first, low byte second)
                                        let _ = nb::block!(uart.write((crc >> 8) as u8));   // High byte
                                        let _ = nb::block!(uart.write((crc & 0xFF) as u8)); // Low byte

                                        // Send \r\n terminator
                                        let _ = nb::block!(uart.write(b'\r'));
                                        let _ = nb::block!(uart.write(b'\n'));

                                        defmt::info!("Binary TX [{}]: {} bytes sent, packet #{}",
                                            trigger_source, total_len, current_seq);

                                        tx_success = true;
                                    }
                                    Err(_) => {
                                        defmt::error!("Binary serialization failed!");
                                    }
                                }
                            });

                            // Transition to WaitingForAck state (outside uart lock)
                            if tx_success {
                                cx.shared.tx_state.lock(|state| {
                                    *state = TxState::WaitingForAck {
                                        seq_num: current_seq,
                                        timeout_counter: ACK_TIMEOUT_SECS,
                                        retry_count: 0,
                                    };
                                });
                                defmt::info!("State: WaitingForAck ({}s timeout)", ACK_TIMEOUT_SECS);
                            }
                        }
                    });
                }
            });
        }
    }

    // UART interrupt: Collect incoming bytes for ACK/NACK parsing
    #[task(binds = UART4, shared = [lora_uart, tx_state], local = [rx_buffer])]
    fn uart4_handler(mut cx: uart4_handler::Context) {
        let mut ack_packet: Option<AckPacket> = None;

        // Collect bytes and parse (inside uart lock)
        cx.shared.lora_uart.lock(|uart| {
            // Collect bytes into buffer
            while let Ok(byte) = uart.read() {
                if cx.local.rx_buffer.push(byte).is_err() {
                    defmt::warn!("N1 RX buffer full, clearing");
                    cx.local.rx_buffer.clear();
                }

                // Check for complete message (ends with \r\n)
                if byte == b'\n' && cx.local.rx_buffer.len() >= 2 {
                    let len = cx.local.rx_buffer.len();
                    if cx.local.rx_buffer[len - 2] == b'\r' {
                        // Complete message received
                        defmt::info!("N1 UART: {} bytes received", cx.local.rx_buffer.len());

                        // Try to parse ACK/NACK
                        ack_packet = parse_ack_message(cx.local.rx_buffer.as_slice());

                        // Clear buffer for next message
                        cx.local.rx_buffer.clear();
                    }
                }
            }

            // Check and clear error flags
            let uart_ptr = unsafe { &*pac::UART4::ptr() };
            let sr = uart_ptr.sr().read();

            if sr.ore().bit_is_set() || sr.nf().bit_is_set() || sr.fe().bit_is_set() {
                let _ = uart_ptr.dr().read();
                defmt::warn!("N1 UART4 errors cleared (ORE={} NF={} FE={})",
                    sr.ore().bit_is_set(), sr.nf().bit_is_set(), sr.fe().bit_is_set());
            }
        });

        // Handle ACK/NACK state transitions (outside uart lock)
        if let Some(ack_pkt) = ack_packet {
            if ack_pkt.msg_type == MSG_TYPE_ACK {
                defmt::info!("ACK received for packet #{}", ack_pkt.seq_num);

                // Check if this ACK matches what we're waiting for
                cx.shared.tx_state.lock(|state| {
                    if let TxState::WaitingForAck { seq_num, .. } = *state {
                        if ack_pkt.seq_num == seq_num {
                            defmt::info!("State: Idle (ACK matched, transmission successful)");
                            *state = TxState::Idle;
                        } else {
                            defmt::warn!("ACK seq mismatch: expected {}, got {}", seq_num, ack_pkt.seq_num);
                        }
                    }
                });
            } else if ack_pkt.msg_type == MSG_TYPE_NACK {
                defmt::warn!("NACK received for packet #{}", ack_pkt.seq_num);

                // NACK means CRC failed - should retry
                cx.shared.tx_state.lock(|state| {
                    if let TxState::WaitingForAck { seq_num, retry_count, .. } = *state {
                        if ack_pkt.seq_num == seq_num {
                            if retry_count < MAX_RETRIES {
                                defmt::warn!("Will retry packet #{}", seq_num);
                                // Reset timeout for retry
                                *state = TxState::WaitingForAck {
                                    seq_num,
                                    timeout_counter: 0, // Trigger immediate retry
                                    retry_count: retry_count + 1,
                                };
                            } else {
                                defmt::error!("Max retries reached after NACK");
                                *state = TxState::Idle;
                            }
                        }
                    }
                });
            }
        }
    }
}