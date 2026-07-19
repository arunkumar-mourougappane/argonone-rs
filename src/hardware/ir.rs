//! Pure decode logic for the IR-learn register protocol
//! (`hardware::i2c`'s reconstruction of register `0x82`), pulled out of
//! that Linux-only module so it's unit-testable on any dev machine —
//! same reasoning `board::probe_with_retries` and `lockfile` already
//! use for their own testability.

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const IR_CODE_LEN: usize = 4;

/// Written to the register to start the listen window, and checked
/// against on read-back to tell "the MCU never overwrote this" apart
/// from "the MCU captured a code". This used to be all-zero, but a real
/// IR remote can legitimately produce an all-zero 32-bit code — that
/// made a genuinely-captured zero code indistinguishable from nothing
/// having been captured at all. A fixed non-zero pattern like this one
/// isn't a *guarantee* no remote ever produces it, but it's the same
/// kind of best-effort choice the rest of this unverified protocol
/// reconstruction already makes.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub const IR_LEARN_SENTINEL: [u8; IR_CODE_LEN] = [0xDE, 0xAD, 0xBE, 0xEF];

/// `None` if the read-back is the wrong length, or still exactly the
/// sentinel written before listening (the MCU never overwrote it —
/// nothing captured); `Some` otherwise, including a genuinely-captured
/// all-zero code.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn decode_learned_ir_code(bytes: &[u8]) -> Option<u32> {
    let bytes: [u8; IR_CODE_LEN] = bytes.try_into().ok()?;
    (bytes != IR_LEARN_SENTINEL).then(|| u32::from_be_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_learned_ir_code_reports_no_capture_when_the_sentinel_is_unchanged() {
        assert_eq!(decode_learned_ir_code(&IR_LEARN_SENTINEL), None);
    }

    #[test]
    fn decode_learned_ir_code_reports_a_genuinely_captured_all_zero_code() {
        // The bug: this used to be indistinguishable from "nothing
        // captured" because the old sentinel was itself all-zero.
        assert_eq!(decode_learned_ir_code(&[0, 0, 0, 0]), Some(0));
    }

    #[test]
    fn decode_learned_ir_code_decodes_a_real_code_big_endian() {
        assert_eq!(
            decode_learned_ir_code(&[0x12, 0x34, 0x56, 0x78]),
            Some(0x1234_5678)
        );
    }

    #[test]
    fn decode_learned_ir_code_rejects_the_wrong_length() {
        assert_eq!(decode_learned_ir_code(&[1, 2, 3]), None);
        assert_eq!(decode_learned_ir_code(&[1, 2, 3, 4, 5]), None);
    }
}
