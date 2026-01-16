// This is the decryption algorithm used by TP-Link.

pub fn decode(buf: &mut [u8]) {
    let mut key = 171u8;

    for b in buf.iter_mut() {
        let tmp = *b;

        *b ^= key;
        key = tmp;
    }
}

// This is the encryption algorithm used by TP-Link.

pub fn encode(buf: &mut [u8]) {
    let mut key = 171u8;

    for b in buf.iter_mut() {
        key ^= *b;
        *b = key;
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_crypt() {
        let buf = [1u8, 2u8, 3u8, 4u8, 5u8];
        let mut enc = vec![];

        enc.extend_from_slice(&buf);
        encode(&mut enc);
        decode(&mut enc);
        assert_eq!(&buf, &enc[..]);
    }
}
