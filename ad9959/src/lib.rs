#![no_std]

use bit_field::BitField;
use bitflags::bitflags;
use embedded_hal::{blocking::delay::DelayUs, digital::v2::OutputPin};

/// A device driver for the AD9959 direct digital synthesis (DDS) chip.
///
/// This chip provides four independently controllable digital-to-analog output sinusoids with
/// configurable phase, amplitude, and frequency. All channels are inherently synchronized as they
/// are derived off a common system clock.
///
/// The chip contains a configurable PLL and supports system clock frequencies up to 500 MHz.
///
/// The chip supports a number of serial interfaces to improve data throughput, including normal,
/// dual, and quad SPI configurations.
pub struct Ad9959<INTERFACE> {
    interface: INTERFACE,
    reference_clock_frequency: f32,
    system_clock_multiplier: u8,
    communication_mode: Mode,
}

/// A trait that allows a HAL to provide a means of communicating with the AD9959.
pub trait Interface {
    type Error;

    fn configure_mode(&mut self, mode: Mode) -> Result<(), Self::Error>;

    fn write(&mut self, addr: u8, data: &[u8]) -> Result<(), Self::Error>;

    fn read(&mut self, addr: u8, dest: &mut [u8]) -> Result<(), Self::Error>;
}

/// Indicates various communication modes of the DDS. The value of this enumeration is equivalent to
/// the configuration bits of the DDS CSR register.
#[derive(Copy, Clone, PartialEq)]
#[repr(u8)]
pub enum Mode {
    SingleBitTwoWire = 0b000,
    SingleBitThreeWire = 0b010,
    TwoBitSerial = 0b100,
    FourBitSerial = 0b110,
}

bitflags! {
    /// Specifies an output channel of the AD9959 DDS chip.
    pub struct Channel: u8 {
        const ONE   = 0b00010000;
        const TWO   = 0b00100000;
        const THREE = 0b01000000;
        const FOUR  = 0b10000000;
        const ALL   = Self::ONE.bits | Self::TWO.bits | Self::THREE.bits | Self::FOUR.bits;
    }
}

/// The configuration registers within the AD9959 DDS device. The values of each register are
/// equivalent to the address.
#[allow(clippy::upper_case_acronyms)]
#[repr(u8)]
pub enum Register {
    CSR = 0x00,
    FR1 = 0x01,
    FR2 = 0x02,
    CFR = 0x03,
    CFTW0 = 0x04,
    CPOW0 = 0x05,
    ACR = 0x06,
    LSRR = 0x07,
    RDW = 0x08,
    FDW = 0x09,
    CW1 = 0x0a,
    CW2 = 0x0b,
    CW3 = 0x0c,
    CW4 = 0x0d,
    CW5 = 0x0e,
    CW6 = 0x0f,
    CW7 = 0x10,
    CW8 = 0x11,
    CW9 = 0x12,
    CW10 = 0x13,
    CW11 = 0x14,
    CW12 = 0x15,
    CW13 = 0x16,
    CW14 = 0x17,
    CW15 = 0x18,
}

/// Possible errors generated by the AD9959 driver.
#[derive(Debug)]
pub enum Error {
    Interface,
    Check,
    Bounds,
    Pin,
    Frequency,
}

impl<I: Interface> Ad9959<I> {
    /// Construct and initialize the DDS.
    ///
    /// Args:
    /// * `interface` - An interface to the DDS.
    /// * `reset_pin` - A pin connected to the DDS reset input.
    /// * `io_update` - A pin connected to the DDS io_update input.
    /// * `delay` - A delay implementation for blocking operation for specific amounts of time.
    /// * `desired_mode` - The desired communication mode of the interface to the DDS.
    /// * `clock_frequency` - The clock frequency of the reference clock input.
    /// * `multiplier` - The desired clock multiplier for the system clock. This multiplies
    ///   `clock_frequency` to generate the system clock.
    pub fn new(
        interface: I,
        mut reset_pin: impl OutputPin,
        io_update: &mut impl OutputPin,
        delay: &mut impl DelayUs<u8>,
        desired_mode: Mode,
        clock_frequency: f32,
        multiplier: u8,
    ) -> Result<Self, Error> {
        let mut ad9959 = Ad9959 {
            interface,
            reference_clock_frequency: clock_frequency,
            system_clock_multiplier: 1,
            communication_mode: desired_mode,
        };

        io_update.set_low().or(Err(Error::Pin))?;

        // Reset the AD9959
        reset_pin.set_high().or(Err(Error::Pin))?;

        // Delay for at least 1 SYNC_CLK period for the reset to occur. The SYNC_CLK is guaranteed
        // to be at least 250KHz (1/4 of 1MHz minimum REF_CLK). We use 5uS instead of 4uS to
        // guarantee conformance with datasheet requirements.
        delay.delay_us(5);

        reset_pin.set_low().or(Err(Error::Pin))?;

        ad9959
            .interface
            .configure_mode(Mode::SingleBitTwoWire)
            .or(Err(Error::Interface))?;

        // Program the interface configuration in the AD9959. Default to all channels enabled.
        let csr = [Channel::ALL.bits() | desired_mode as u8];
        ad9959.write(Register::CSR, &csr)?;

        // Latch the new interface configuration.
        io_update.set_high().or(Err(Error::Pin))?;

        // Delay for at least 1 SYNC_CLK period for the update to occur. The SYNC_CLK is guaranteed
        // to be at least 250KHz (1/4 of 1MHz minimum REF_CLK). We use 5uS instead of 4uS to
        // guarantee conformance with datasheet requirements.
        delay.delay_us(5);

        io_update.set_low().or(Err(Error::Pin))?;

        ad9959
            .interface
            .configure_mode(desired_mode)
            .or(Err(Error::Interface))?;

        // Empirical evidence indicates a delay is necessary here for the IO update to become
        // active. This is likely due to needing to wait at least 1 clock cycle of the DDS for the
        // interface update to occur.
        // Delay for at least 1 SYNC_CLK period for the update to occur. The SYNC_CLK is guaranteed
        // to be at least 250KHz (1/4 of 1MHz minimum REF_CLK). We use 5uS instead of 4uS to
        // guarantee conformance with datasheet requirements.
        delay.delay_us(5);

        // Read back the CSR to ensure it specifies the mode correctly.
        let mut updated_csr: [u8; 1] = [0];
        ad9959.read(Register::CSR, &mut updated_csr)?;
        if updated_csr[0] != csr[0] {
            return Err(Error::Check);
        }

        // Set the clock frequency to configure the device as necessary.
        ad9959.configure_system_clock(clock_frequency, multiplier)?;

        // Latch the new clock configuration.
        io_update.set_high().or(Err(Error::Pin))?;

        // Delay for at least 1 SYNC_CLK period for the update to occur. The SYNC_CLK is guaranteed
        // to be at least 250KHz (1/4 of 1MHz minimum REF_CLK). We use 5uS instead of 4uS to
        // guarantee conformance with datasheet requirements.
        delay.delay_us(5);

        io_update.set_low().or(Err(Error::Pin))?;

        Ok(ad9959)
    }

    fn read(&mut self, reg: Register, data: &mut [u8]) -> Result<(), Error> {
        self.interface
            .read(reg as u8, data)
            .or(Err(Error::Interface))
    }

    fn write(&mut self, reg: Register, data: &[u8]) -> Result<(), Error> {
        self.interface
            .write(reg as u8, data)
            .or(Err(Error::Interface))
    }

    /// Configure the internal system clock of the chip.
    ///
    /// Arguments:
    /// * `reference_clock_frequency` - The reference clock frequency provided to the AD9959 core.
    /// * `multiplier` - The frequency multiplier of the system clock. Must be 1 or 4-20.
    ///
    /// Returns:
    /// The actual frequency configured for the internal system clock.
    fn configure_system_clock(
        &mut self,
        reference_clock_frequency: f32,
        multiplier: u8,
    ) -> Result<f32, Error> {
        let frequency =
            validate_clocking(reference_clock_frequency, multiplier)?;
        self.reference_clock_frequency = reference_clock_frequency;

        // TODO: Update / disable any enabled channels?
        let mut fr1: [u8; 3] = [0, 0, 0];
        self.read(Register::FR1, &mut fr1)?;
        fr1[0].set_bits(2..=6, multiplier);

        let vco_range = frequency > 200e6;
        fr1[0].set_bit(7, vco_range);

        self.write(Register::FR1, &fr1)?;
        self.system_clock_multiplier = multiplier;

        Ok(self.system_clock_frequency())
    }

    /// Get the current reference clock frequency in Hz.
    pub fn get_reference_clock_frequency(&self) -> f32 {
        self.reference_clock_frequency
    }

    /// Get the current reference clock multiplier.
    pub fn get_reference_clock_multiplier(&mut self) -> Result<u8, Error> {
        let mut fr1: [u8; 3] = [0, 0, 0];
        self.read(Register::FR1, &mut fr1)?;

        Ok(fr1[0].get_bits(2..=6) as u8)
    }

    /// Perform a self-test of the communication interface.
    ///
    /// Note:
    /// This modifies the existing channel enables. They are restored upon exit.
    ///
    /// Returns:
    /// True if the self test succeeded. False otherwise.
    pub fn self_test(&mut self) -> Result<bool, Error> {
        let mut csr: [u8; 1] = [0];
        self.read(Register::CSR, &mut csr)?;
        let old_csr = csr[0];

        // Enable all channels.
        csr[0].set_bits(4..8, 0xF);
        self.write(Register::CSR, &csr)?;

        // Read back the enable.
        csr[0] = 0;
        self.read(Register::CSR, &mut csr)?;
        if csr[0].get_bits(4..8) != 0xF {
            return Ok(false);
        }

        // Clear all channel enables.
        csr[0].set_bits(4..8, 0x0);
        self.write(Register::CSR, &csr)?;

        // Read back the enable.
        csr[0] = 0xFF;
        self.read(Register::CSR, &mut csr)?;
        if csr[0].get_bits(4..8) != 0 {
            return Ok(false);
        }

        // Restore the CSR.
        csr[0] = old_csr;
        self.write(Register::CSR, &csr)?;

        Ok(true)
    }

    /// Get the current system clock frequency in Hz.
    fn system_clock_frequency(&self) -> f32 {
        self.system_clock_multiplier as f32
            * self.reference_clock_frequency as f32
    }

    /// Update an output channel configuration register.
    ///
    /// Args:
    /// * `channel` - The channel to configure.
    /// * `register` - The register to update.
    /// * `data` - The contents to write to the provided register.
    fn modify_channel(
        &mut self,
        channel: Channel,
        register: Register,
        data: &[u8],
    ) -> Result<(), Error> {
        // Disable all other outputs so that we can update the configuration register of only the
        // specified channel.
        let csr = [self.communication_mode as u8 | channel.bits()];

        self.write(Register::CSR, &csr)?;
        self.write(register, data)?;

        Ok(())
    }

    /// Read a configuration register of a specific channel.
    ///
    /// Args:
    /// * `channel` - The channel to read.
    /// * `register` - The register to read.
    /// * `data` - A location to store the read register contents.
    fn read_channel(
        &mut self,
        channel: Channel,
        register: Register,
        data: &mut [u8],
    ) -> Result<(), Error> {
        // Disable all other channels in the CSR so that we can read the configuration register of
        // only the desired channel.
        let mut csr = [0];
        self.read(Register::CSR, &mut csr)?;
        let new_csr = [self.communication_mode as u8 | channel.bits()];

        self.write(Register::CSR, &new_csr)?;
        self.read(register, data)?;

        // Restore the previous CSR. Note that the re-enable of the channel happens immediately, so
        // the CSR update does not need to be latched.
        self.write(Register::CSR, &csr)?;

        Ok(())
    }

    /// Configure the phase of a specified channel.
    ///
    /// Arguments:
    /// * `channel` - The channel to configure the frequency of.
    /// * `phase_turns` - The desired phase offset in turns.
    ///
    /// Returns:
    /// The actual programmed phase offset of the channel in turns.
    pub fn set_phase(
        &mut self,
        channel: Channel,
        phase_turns: f32,
    ) -> Result<f32, Error> {
        let phase_offset = phase_to_pow(phase_turns)?;
        self.modify_channel(
            channel,
            Register::CPOW0,
            &phase_offset.to_be_bytes(),
        )?;

        Ok((phase_offset as f32) / ((1 << 14) as f32))
    }

    /// Get the current phase of a specified channel.
    ///
    /// Args:
    /// * `channel` - The channel to get the phase of.
    ///
    /// Returns:
    /// The phase of the channel in turns.
    pub fn get_phase(&mut self, channel: Channel) -> Result<f32, Error> {
        let mut phase_offset: [u8; 2] = [0; 2];
        self.read_channel(channel, Register::CPOW0, &mut phase_offset)?;

        let phase_offset = u16::from_be_bytes(phase_offset) & 0x3FFFu16;

        Ok((phase_offset as f32) / ((1 << 14) as f32))
    }

    /// Configure the amplitude of a specified channel.
    ///
    /// Arguments:
    /// * `channel` - The channel to configure the frequency of.
    /// * `amplitude` - A normalized amplitude setting [0, 1].
    ///
    /// Returns:
    /// The actual normalized amplitude of the channel relative to full-scale range.
    pub fn set_amplitude(
        &mut self,
        channel: Channel,
        amplitude: f32,
    ) -> Result<f32, Error> {
        let acr = amplitude_to_acr(amplitude)?;
        let amplitude = if (acr & (1 << 12)) != 0 {
            // Isolate the amplitude scaling factor from ACR
            (acr & ((1 << 10) - 1)) as f32 / (1 << 10) as f32
        } else {
            // Amplitude is always at full-scale with amplitude multiplier disabled
            1.0
        };
        // ACR is a 24-bits register, the MSB of the calculated ACR must be discarded.
        self.modify_channel(channel, Register::ACR, &acr.to_be_bytes()[1..])?;

        Ok(amplitude)
    }

    /// Get the configured amplitude of a channel.
    ///
    /// Args:
    /// * `channel` - The channel to get the amplitude of.
    ///
    /// Returns:
    /// The normalized amplitude of the channel.
    pub fn get_amplitude(&mut self, channel: Channel) -> Result<f32, Error> {
        let mut acr: [u8; 3] = [0; 3];
        self.read_channel(channel, Register::ACR, &mut acr)?;

        if acr[1].get_bit(4) {
            let amplitude_control: u16 =
                (((acr[1] as u16) << 8) | (acr[2] as u16)) & 0x3FF;
            Ok(amplitude_control as f32 / (1 << 10) as f32)
        } else {
            Ok(1.0)
        }
    }

    /// Configure the frequency of a specified channel.
    ///
    /// Arguments:
    /// * `channel` - The channel to configure the frequency of.
    /// * `frequency` - The desired output frequency in Hz.
    ///
    /// Returns:
    /// The actual programmed frequency of the channel.
    pub fn set_frequency(
        &mut self,
        channel: Channel,
        frequency: f32,
    ) -> Result<f32, Error> {
        let tuning_word =
            frequency_to_ftw(frequency, self.system_clock_frequency())?;

        self.modify_channel(
            channel,
            Register::CFTW0,
            &tuning_word.to_be_bytes(),
        )?;
        Ok((tuning_word as f32 / (1u64 << 32) as f32)
            * self.system_clock_frequency())
    }

    /// Get the frequency of a channel.
    ///
    /// Arguments:
    /// * `channel` - The channel to get the frequency of.
    ///
    /// Returns:
    /// The frequency of the channel in Hz.
    pub fn get_frequency(&mut self, channel: Channel) -> Result<f32, Error> {
        // Read the frequency tuning word for the channel.
        let mut tuning_word: [u8; 4] = [0; 4];
        self.read_channel(channel, Register::CFTW0, &mut tuning_word)?;
        let tuning_word = u32::from_be_bytes(tuning_word);

        // Convert the tuning word into a frequency.
        Ok((tuning_word as f32 * self.system_clock_frequency())
            / (1u64 << 32) as f32)
    }

    /// Finalize DDS configuration
    ///
    /// # Note
    /// This is intended for when the DDS profiles will be written as a stream of data to the DDS.
    ///
    /// # Returns
    /// (i, mode) where `i` is the interface to the DDS and `mode` is the frozen `Mode`.
    pub fn freeze(self) -> (I, Mode) {
        (self.interface, self.communication_mode)
    }
}

/// Validate the internal system clock configuration of the chip.
///
/// Arguments:
/// * `reference_clock_frequency` - The reference clock frequency provided to the AD9959 core.
/// * `multiplier` - The frequency multiplier of the system clock. Must be 1 or 4-20.
///
/// Returns:
/// The system clock frequency to be configured.
pub fn validate_clocking(
    reference_clock_frequency: f32,
    multiplier: u8,
) -> Result<f32, Error> {
    if multiplier != 1 && !(4..=20).contains(&multiplier)
        || (multiplier != 1 && reference_clock_frequency < 10e6)
        || reference_clock_frequency < 1e6
    {
        return Err(Error::Bounds);
    }

    let frequency = multiplier as f32 * reference_clock_frequency;
    if !(255e6..=500e6).contains(&frequency)
        && !(100e6..=160e6).contains(&frequency)
        && (multiplier != 1 || !(0.0..100e6).contains(&frequency))
    {
        return Err(Error::Frequency);
    }

    Ok(frequency)
}

pub fn frequency_to_ftw(
    dds_frequency: f32,
    system_clock_frequency: f32,
) -> Result<u32, Error> {
    if !(0.0..=(system_clock_frequency / 2.0)).contains(&dds_frequency) {
        return Err(Error::Bounds);
    }
    // The function for channel frequency is `f_out = FTW * f_s / 2^32`, where FTW is the
    // frequency tuning word and f_s is the system clock rate.
    Ok(((dds_frequency / system_clock_frequency) * (1u64 << 32) as f32) as u32)
}

pub fn phase_to_pow(phase_turns: f32) -> Result<u16, Error> {
    Ok((phase_turns * (1 << 14) as f32) as u16 & ((1 << 14) - 1))
}

pub fn amplitude_to_acr(amplitude: f32) -> Result<u32, Error> {
    if !(0.0..=1.0).contains(&amplitude) {
        return Err(Error::Bounds);
    }

    let amplitude_control: u16 = (amplitude * (1 << 10) as f32) as u16;

    // Enable the amplitude multiplier for the channel if required. The amplitude control has
    // full-scale at 0x3FF (amplitude of 1), so the multiplier should be disabled whenever
    // full-scale is used.
    let acr = if amplitude_control < (1 << 10) {
        // Enable the amplitude multiplier
        (amplitude_control & 0x3FF) | (1 << 12)
    } else {
        0
    };

    Ok(acr as u32)
}

/// Represents a means of serializing a DDS profile for writing to a stream.
pub struct ProfileSerializer {
    // heapless::Vec<u8, 32>, especially its extend_from_slice() is slow
    data: [u8; 32],
    index: usize,
    // make mode u32 to work around https://github.com/japaric/heapless/issues/305
    mode: u32,
}

impl ProfileSerializer {
    /// Construct a new serializer.
    ///
    /// # Args
    /// * `mode` - The communication mode of the DDS.
    pub fn new(mode: Mode) -> Self {
        Self {
            mode: mode as _,
            data: [0; 32],
            index: 0,
        }
    }

    /// Update a number of channels with the requested profile.
    ///
    /// # Args
    /// * `channels` - A set of channels to apply the configuration to.
    /// * `ftw` - If provided, indicates a frequency tuning word for the channels.
    /// * `pow` - If provided, indicates a phase offset word for the channels.
    /// * `acr` - If provided, indicates the amplitude control register for the channels. The ACR
    ///   should be stored in the 3 LSB of the word. Note that if amplitude scaling is to be used,
    ///   the "Amplitude multiplier enable" bit must be set.
    #[inline]
    pub fn update_channels(
        &mut self,
        channels: Channel,
        ftw: Option<u32>,
        pow: Option<u16>,
        acr: Option<u32>,
    ) {
        let csr = [self.mode as u8 | channels.bits()];
        self.add_write(Register::CSR, &csr);

        if let Some(ftw) = ftw {
            self.add_write(Register::CFTW0, &ftw.to_be_bytes());
        }

        if let Some(pow) = pow {
            self.add_write(Register::CPOW0, &pow.to_be_bytes());
        }

        if let Some(acr) = acr {
            self.add_write(Register::ACR, &acr.to_be_bytes()[1..]);
        }
    }

    /// Update the system clock configuration.
    ///
    /// # Args
    /// * `reference_clock_frequency` - The reference clock frequency provided to the AD9959 core.
    /// * `multiplier` - The frequency multiplier of the system clock. Must be 1 or 4-20.
    pub fn set_system_clock(
        &mut self,
        reference_clock_frequency: f32,
        multiplier: u8,
    ) -> Result<f32, Error> {
        let frequency = reference_clock_frequency * multiplier as f32;

        // The enabled channel will be updated after clock reconfig
        let mut fr1 = [0u8; 3];

        // The ad9959 crate does not modify FR1[0:17]. These bits keep their default value.
        // These bits by default are 0.
        fr1[0].set_bits(2..=6, multiplier);

        // Frequencies within the VCO forbidden range (160e6, 255e6) are already rejected.
        let vco_range = frequency > 200e6;
        fr1[0].set_bit(7, vco_range);

        self.add_write(Register::FR1, &fr1);
        Ok(frequency)
    }

    /// Add a register write to the serialization data.
    fn add_write(&mut self, register: Register, value: &[u8]) {
        let data = &mut self.data[self.index..];
        data[0] = register as u8;
        data[1..][..value.len()].copy_from_slice(value);
        self.index += value.len() + 1;
    }

    #[inline]
    fn pad(&mut self) {
        // Pad the buffer to 32-bit (4 byte) alignment by adding dummy writes to CSR and LSRR.
        // In the case of 1 byte padding, this instead pads with 5 bytes as there is no
        // valid single-byte write that could be used.
        if self.index & 1 != 0 {
            // Pad with 3 bytes
            self.add_write(Register::LSRR, &[0, 0]);
        }
        if self.index & 2 != 0 {
            // Pad with 2 bytes
            self.add_write(Register::CSR, &[self.mode as _]);
        }
        debug_assert_eq!(self.index & 3, 0);
    }

    /// Get the serialized profile as a slice of 32-bit words.
    ///
    /// # Note
    /// The serialized profile will be padded to the next 32-bit word boundary by adding dummy
    /// writes to the CSR or LSRR registers.
    ///
    /// # Returns
    /// A slice of `u32` words representing the serialized profile.
    #[inline]
    pub fn finalize(&mut self) -> &[u32] {
        self.pad();
        bytemuck::cast_slice(&self.data[..self.index])
    }
}

/// Represents a fully defined DDS profile, with parameters expressed in machine units
pub struct Profile {
    pub ftw: u32,
    pub pow: u16,
    pub acr: u32,
}
