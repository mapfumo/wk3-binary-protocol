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
        timer::{CounterHz, Event},
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

    // --- Configuration Constants ---
    const NODE_ID: &str = "N2";              // Node identifier for display

    // UART RX buffer size - sized for RYLR998 capabilities
    // RYLR998 supports 240-byte payloads (NOT LoRaWAN's 51-byte limit!)
    // RX format: "+RCV=<addr>,<len>,<data>,<rssi>,<snr>\r\n"
    // Example: "+RCV=1,240,<240 bytes>,-20,12\r\n" = ~265 bytes total
    // 255 bytes gives headroom for current payloads (~44 bytes) plus future expansion
    const RX_BUFFER_SIZE: usize = 255;

    const NETWORK_ID: u8 = 18;               // LoRa network ID
    const LORA_FREQ: u32 = 915;              // LoRa frequency in MHz (915 for US)

    // --- Binary Protocol Data Structures ---
    use serde::{Serialize, Deserialize};

    /// Sensor data packet for binary transmission (must match Node 1)
    #[derive(Debug, Clone, Copy, Serialize, Deserialize)]
    pub struct SensorDataPacket {
        pub seq_num: u16,           // Sequence number for duplicate detection
        pub temperature: i16,       // Temperature in centidegrees (e.g., 2710 = 27.1°C)
        pub humidity: u16,          // Humidity in basis points (e.g., 5600 = 56.0%)
        pub gas_resistance: u32,    // Gas resistance in ohms
    }

    /// ACK/NACK packet for acknowledgment (must match Node 1)
    /// Size: 3 bytes (1 byte msg_type + 2 bytes seq_num)
    #[derive(Debug, Clone, Copy, Serialize, Deserialize)]
    pub struct AckPacket {
        pub msg_type: u8,   // 1 = ACK (success), 2 = NACK (CRC failure)
        pub seq_num: u16,   // Which packet we're acknowledging
    }

    // Message type constants
    const MSG_TYPE_ACK: u8 = 1;
    const MSG_TYPE_NACK: u8 = 2;

    /// Calculate CRC-16 checksum for data integrity
    fn calculate_crc16(data: &[u8]) -> u16 {
        use crc::{Crc, CRC_16_IBM_3740};
        const CRC16: Crc<u16> = Crc::<u16>::new(&CRC_16_IBM_3740);
        CRC16.checksum(data)
    }

    /// Send ACK packet to Node 1
    /// Format: AT+SEND=1,<length>,<binary_ack_packet>\r\n
    fn send_ack(uart: &mut Serial<pac::UART4>, seq_num: u16, is_ack: bool) {
        use heapless::String;
        use core::fmt::Write;

        let ack_packet = AckPacket {
            msg_type: if is_ack { MSG_TYPE_ACK } else { MSG_TYPE_NACK },
            seq_num,
        };

        // Serialize ACK packet
        let mut ack_buffer = [0u8; 8];
        match postcard::to_slice(&ack_packet, &mut ack_buffer) {
            Ok(serialized) => {
                let ack_len = serialized.len();

                // Send AT command: AT+SEND=1,<length>,<ack_data>\r\n
                // Address 1 = Node 1 (sender)
                let cmd_prefix = "AT+SEND=1,";
                for b in cmd_prefix.as_bytes() {
                    let _ = nb::block!(uart.write(*b));
                }

                // Send length as ASCII
                let mut len_str: String<8> = String::new();
                let _ = core::write!(len_str, "{},", ack_len);
                for b in len_str.as_bytes() {
                    let _ = nb::block!(uart.write(*b));
                }

                // Send binary ACK payload
                for b in serialized {
                    let _ = nb::block!(uart.write(*b));
                }

                // Send \r\n terminator
                let _ = nb::block!(uart.write(b'\r'));
                let _ = nb::block!(uart.write(b'\n'));

                defmt::info!("{} sent for packet #{}",
                    if is_ack { "ACK" } else { "NACK" }, seq_num);
            }
            Err(_) => {
                defmt::error!("Failed to serialize ACK packet");
            }
        }
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
    type BusManager = shared_bus::BusManager<CortexMMutex<I2cCompat<MyI2c>>>;
    type I2cProxy = shared_bus::I2cProxy<'static, CortexMMutex<I2cCompat<MyI2c>>>;

    type LoraDisplay = Ssd1306<I2CInterface<I2cProxy>, DisplaySize128x64, BufferedGraphicsMode<DisplaySize128x64>>;

    #[derive(Debug, Clone, Copy)]
    pub struct SensorData {
        pub temperature: f32,
        pub humidity: f32,
        pub gas_resistance: u32,
        pub packet_num: u16,
    }

    #[shared]
    struct Shared {
        lora_uart: Serial<pac::UART4>,
        display: LoraDisplay,
        last_packet: Option<ParsedMessage>,
        packets_received: u32,
    }

    #[local]
    struct Local {
        led: Pin<'A', 5, Output>,
        timer: CounterHz<pac::TIM2>,
        rx_buffer: Vec<u8, RX_BUFFER_SIZE>,
    }

    #[derive(Debug, Clone, Copy)]
    pub struct ParsedMessage {
        pub sensor_data: SensorData,
        pub rssi: i16,
        pub snr: i16,
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

        // 1. Configure RCC clocks
        let mut rcc = dp.RCC.freeze(Config::hsi().sysclk(84.MHz()));

        // 2. Split GPIOs
        let gpioa = dp.GPIOA.split(&mut rcc);
        let gpiob = dp.GPIOB.split(&mut rcc);
        let gpioc = dp.GPIOC.split(&mut rcc);

        let led = gpioa.pa5.into_push_pull_output();

        // --- UART4 for LoRa ---
        let tx = gpioc.pc10.into_alternate();
        let rx = gpioc.pc11.into_alternate();
        let mut lora_uart = Serial::new(
            dp.UART4,
            (tx, rx),
            SerialConfig::default().baudrate(115200.bps()),
            &mut rcc
        ).unwrap();

        // Configure LoRa module before enabling RX interrupt
        defmt::info!("Configuring LoRa module (Node 2)...");
        send_at_command(&mut lora_uart, "AT");
        send_at_command(&mut lora_uart, "AT+ADDRESS=2");

        let mut cmd_buf: String<32> = String::new();
        let _ = core::write!(cmd_buf, "AT+NETWORKID={}", NETWORK_ID);
        send_at_command(&mut lora_uart, cmd_buf.as_str());

        cmd_buf.clear();
        let _ = core::write!(cmd_buf, "AT+BAND={}000000", LORA_FREQ);
        send_at_command(&mut lora_uart, cmd_buf.as_str());

        send_at_command(&mut lora_uart, "AT+PARAMETER=7,9,1,7");

        // Flush any pending responses from configuration BEFORE enabling interrupt
        while lora_uart.read().is_ok() {}

        // Explicitly clear any error flags (especially ORE) before enabling interrupt
        let uart_ptr = unsafe { &*pac::UART4::ptr() };
        let sr = uart_ptr.sr().read();
        if sr.ore().bit_is_set() || sr.nf().bit_is_set() || sr.fe().bit_is_set() {
            let _ = uart_ptr.dr().read();
            defmt::info!("N2 INIT: Cleared error flags (ORE={} NF={} FE={})",
                sr.ore().bit_is_set(), sr.nf().bit_is_set(), sr.fe().bit_is_set());
        }

        defmt::info!("LoRa module configured");
        lora_uart.listen(SerialEvent::RxNotEmpty);

        // --- I2C1 for Display ---
        let scl = gpiob.pb8.into_alternate_open_drain();
        let sda = gpiob.pb9.into_alternate_open_drain();
        let i2c = I2c::new(dp.I2C1, (scl, sda), 100.kHz(), &mut rcc);

        let i2c_compat = I2cCompat(i2c);
        let bus: &'static BusManager = shared_bus::new_cortexm!(I2cCompat<MyI2c> = i2c_compat).unwrap();

        // --- Display ---
        let interface = I2CInterface::new(bus.acquire_i2c(), 0x3C, 0x40);
        let mut display = Ssd1306::new(interface, DisplaySize128x64, DisplayRotation::Rotate0)
            .into_buffered_graphics_mode();
        display.init().unwrap();

        // Initial display message
        let style = MonoTextStyleBuilder::new()
            .font(&FONT_6X10)
            .text_color(BinaryColor::On)
            .build();
        let _ = display.clear(BinaryColor::Off);
        Text::new("N2 RECEIVER", Point::new(0, 8), style).draw(&mut display).ok();

        let mut init_buf: String<32> = String::new();
        let _ = core::write!(init_buf, "Net:{} {}MHz", NETWORK_ID, LORA_FREQ);
        Text::new(&init_buf, Point::new(0, 20), style).draw(&mut display).ok();

        Text::new("Waiting...", Point::new(0, 32), style).draw(&mut display).ok();
        let _ = display.flush();

        // --- Timer for LED blinking ---
        let mut timer = dp.TIM2.counter_hz(&mut rcc);
        timer.start(2.Hz()).unwrap();  // 2 Hz for heartbeat
        timer.listen(Event::Update);

        (
            Shared {
                lora_uart,
                display,
                last_packet: None,
                packets_received: 0,
            },
            Local {
                led,
                timer,
                rx_buffer: Vec::new(),
            },
            init::Monotonics()
        )
    }

    #[task(binds = TIM2, shared = [display, last_packet, packets_received], local = [led, timer])]
    fn tim2_handler(mut cx: tim2_handler::Context) {
        cx.local.timer.clear_flags(stm32f4xx_hal::timer::Flag::Update);
        cx.local.led.toggle();

        // Copy packet data quickly while holding lock
        let packet_copy = cx.shared.last_packet.lock(|pkt_opt| *pkt_opt);
        let total_count = cx.shared.packets_received.lock(|count| *count);

        defmt::info!("N2 Timer: total_count={}, has_packet={}", total_count, packet_copy.is_some());

        // Update display OUTSIDE locks (slow I2C is OK here in timer context)
        if let Some(parsed) = packet_copy {
            cx.shared.display.lock(|disp| {
                let _ = disp.clear(BinaryColor::Off);
                let style = MonoTextStyleBuilder::new()
                    .font(&FONT_6X10)
                    .text_color(BinaryColor::On)
                    .build();

                let mut buf: String<64> = String::new();

                // Line 1: Temperature & Humidity
                let _ = core::write!(buf, "T:{:.1}C H:{:.0}%",
                    parsed.sensor_data.temperature, parsed.sensor_data.humidity);
                Text::new(&buf, Point::new(0, 8), style).draw(disp).ok();

                buf.clear();
                // Line 2: Gas resistance
                let _ = core::write!(buf, "Gas:{:.0}k",
                    parsed.sensor_data.gas_resistance as f32 / 1000.0);
                Text::new(&buf, Point::new(0, 20), style).draw(disp).ok();

                buf.clear();
                // Line 3: Node ID and packet info
                let _ = core::write!(buf, "{} RX #{:04}",
                    NODE_ID, parsed.sensor_data.packet_num);
                Text::new(&buf, Point::new(0, 32), style).draw(disp).ok();

                buf.clear();
                // Line 4: Network ID and frequency
                let _ = core::write!(buf, "Net:{} {}MHz",
                    NETWORK_ID, LORA_FREQ);
                Text::new(&buf, Point::new(0, 44), style).draw(disp).ok();

                buf.clear();
                // Line 5: RSSI and SNR with total count
                let _ = core::write!(buf, "RSSI:{} SNR:{} #{}",
                    parsed.rssi, parsed.snr, total_count);
                Text::new(&buf, Point::new(0, 56), style).draw(disp).ok();

                let _ = disp.flush();  // Slow I2C flush is safe here
            });
        }
    }

    // UART interrupt handler - Keep it simple!
    //
    // CRITICAL: This interrupt handler MUST be fast and simple.
    // Previous attempts with extensive ORE flag checking, status register logging,
    // and diagnostic code caused data corruption/scrambling.
    //
    // This simpler version from commit 80c7c5e works reliably:
    // 1. Read all available bytes
    // 2. Check for message terminator (\n)
    // 3. Process complete message OUTSIDE the UART lock
    // 4. Clear buffer for next message
    //
    // NO display updates here - those happen in the timer interrupt
    #[task(binds = UART4, shared = [lora_uart, last_packet, packets_received], local = [rx_buffer])]
    fn uart4_handler(mut cx: uart4_handler::Context) {
        // Read ALL available bytes from UART in one interrupt
        let mut should_process = false;
        let mut bytes_read = 0u16;

        cx.shared.lora_uart.lock(|uart| {
            // Drain all available bytes from UART buffer
            while let Ok(byte) = uart.read() {
                bytes_read += 1;
                // Add byte to buffer (with overflow protection)
                if cx.local.rx_buffer.len() < RX_BUFFER_SIZE {
                    let _ = cx.local.rx_buffer.push(byte);
                }
                // Check for complete message (ends with \n)
                if byte == b'\n' {
                    should_process = true;
                }
            }
        });

        // Log that we got UART interrupt and how many bytes
        if bytes_read > 0 {
            defmt::info!("UART INT: {} bytes, complete={}", bytes_read, should_process);
        }

        // Process message OUTSIDE uart lock to allow new interrupts
        if should_process {
            // Debug: log buffer length and attempt to show as text
            defmt::info!("Processing buffer: {} bytes", cx.local.rx_buffer.len());
            if let Ok(msg_text) = core::str::from_utf8(cx.local.rx_buffer.as_slice()) {
                defmt::info!("Buffer as text: {}", msg_text);
            }

            // Parse +RCV message format: +RCV=<Address>,<Length>,<Data>,<RSSI>,<SNR>\r\n
            // The <Data> part is now BINARY (not text), but RSSI/SNR are still text
            if let Some(parsed) = parse_binary_lora_message(cx.local.rx_buffer.as_slice()) {
                defmt::info!("Binary RX - T:{} H:{} G:{} Pkt:{} RSSI:{} SNR:{}",
                    parsed.sensor_data.temperature, parsed.sensor_data.humidity,
                    parsed.sensor_data.gas_resistance, parsed.sensor_data.packet_num,
                    parsed.rssi, parsed.snr);

                // Store parsed data for timer interrupt to display
                cx.shared.last_packet.lock(|last_pkt| {
                    *last_pkt = Some(parsed);
                });

                cx.shared.packets_received.lock(|count| {
                    *count += 1;
                });

                // Send ACK back to Node 1 (CRC validation passed)
                cx.shared.lora_uart.lock(|uart| {
                    send_ack(uart, parsed.sensor_data.packet_num, true);
                });
            } else {
                defmt::warn!("Failed to parse binary message");
            }

            // Clear buffer for next message
            cx.local.rx_buffer.clear();
        }
    }

    /// Parse binary LoRa message from RYLR998
    /// Format: +RCV=<Address>,<Length>,<BinaryData>,<RSSI>,<SNR>\r\n
    /// where <BinaryData> is postcard-serialized SensorDataPacket
    fn parse_binary_lora_message(buffer: &[u8]) -> Option<ParsedMessage> {
        // Check prefix: must start with "+RCV="
        if buffer.len() < 10 || &buffer[0..5] != b"+RCV=" {
            return None;
        }

        // Find first two commas by scanning bytes
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

        // Extract length from between commas (this is ASCII text)
        let len_bytes = &buffer[comma1 + 1..comma2];
        let len_str = core::str::from_utf8(len_bytes).ok()?;
        let payload_len: usize = len_str.parse().ok()?;

        // Binary payload starts after second comma
        let payload_start = comma2 + 1;
        let payload_end = payload_start + payload_len;

        if payload_end > buffer.len() {
            defmt::warn!("Payload exceeds buffer");
            return None;
        }

        let binary_payload = &buffer[payload_start..payload_end];

        // Payload format: [data bytes...][CRC high byte][CRC low byte]
        // Minimum payload: 3 bytes (1 byte data + 2 bytes CRC)
        if binary_payload.len() < 3 {
            defmt::warn!("Payload too short for CRC validation");
            return None;
        }

        // Split payload: data is everything except last 2 bytes
        let data_len = binary_payload.len() - 2;
        let data_bytes = &binary_payload[0..data_len];
        let received_crc_high = binary_payload[data_len];
        let received_crc_low = binary_payload[data_len + 1];
        let received_crc = ((received_crc_high as u16) << 8) | (received_crc_low as u16);

        // Calculate CRC on data portion
        let calculated_crc = calculate_crc16(data_bytes);

        // Validate CRC
        if received_crc != calculated_crc {
            defmt::error!("CRC FAIL! Received: 0x{:04X}, Calculated: 0x{:04X}",
                received_crc, calculated_crc);
            return None;
        }

        defmt::info!("CRC OK: 0x{:04X}", received_crc);

        // Deserialize with postcard (only the data portion, not the CRC)
        let sensor_packet: SensorDataPacket = match postcard::from_bytes(data_bytes) {
            Ok(pkt) => pkt,
            Err(_) => {
                defmt::error!("Postcard deserialization failed");
                return None;
            }
        };

        // Parse RSSI and SNR after the binary payload (this is ASCII text)
        // Format: ,<rssi>,<snr>\r\n
        let after_payload_bytes = &buffer[payload_end..];
        let after_payload_str = core::str::from_utf8(after_payload_bytes).ok()?;

        let parts: Vec<&str, 4> = after_payload_str.split(',').collect();
        if parts.len() < 3 {
            return None;
        }

        let rssi: i16 = parts[1].parse().ok()?;
        let snr: i16 = parts[2].trim().parse().ok()?;

        // Convert from binary format to display format
        let temp_c = sensor_packet.temperature as f32 / 10.0;
        let humid_pct = sensor_packet.humidity as f32 / 100.0;

        Some(ParsedMessage {
            sensor_data: SensorData {
                temperature: temp_c,
                humidity: humid_pct,
                gas_resistance: sensor_packet.gas_resistance,
                packet_num: sensor_packet.seq_num,
            },
            rssi,
            snr,
        })
    }
}
