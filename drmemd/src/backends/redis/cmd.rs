use drmem_api::device;
use std::sync::Arc;

#[derive(Clone)]
pub struct Builder {
    hist_key: Arc<str>,
    scratch: Vec<u8>,
}

impl Builder {
    pub fn new(hist_key: Arc<str>) -> Self {
        Builder {
            hist_key,
            scratch: Vec::new(),
        }
    }

    // Encodes a `device::Value` into a binary which gets stored in
    // redis. This encoding lets us store type information in redis so
    // there's no rounding errors or misinterpretation of the data.

    pub fn to_redis<'a>(val: &device::Value, buf: &'a mut Vec<u8>) -> &'a [u8] {
        match val {
            device::Value::Bool(false) => &[b'B', b'F'],
            device::Value::Bool(true) => &[b'B', b'T'],

            // Integers start with an 'I' followed by 4 bytes.
            device::Value::Int(v) => {
                buf.clear();
                buf.reserve(5usize.saturating_sub(buf.capacity()));

                buf.push(b'I');
                buf.extend_from_slice(&v.to_be_bytes());
                &buf[..]
            }

            // Floating point values start with a 'D' and are followed by
            // 8 bytes.
            device::Value::Flt(v) => {
                buf.clear();
                buf.reserve(9usize.saturating_sub(buf.capacity()));

                buf.push(b'D');
                buf.extend_from_slice(&v.to_be_bytes());
                &buf[..]
            }

            // Strings start with an 'S', followed by a 4-byte length
            // field, and then followed by the string content.
            device::Value::Str(s) => {
                buf.clear();

                let s = s.as_bytes();

                buf.reserve((5 + s.len()).saturating_sub(buf.capacity()));

                buf.push(b'S');
                buf.extend_from_slice(&(s.len() as u32).to_be_bytes());
                buf.extend_from_slice(s);
                &buf[..]
            }

            // Colors start with a 'C', followed by 4 u8 values,
            // representing red, green, blue, and alpha intensities,
            // respectively.
            device::Value::Color(v) => {
                buf.clear();
                buf.reserve(5usize.saturating_sub(buf.capacity()));

                buf.push(b'C');
                buf.push(v.red);
                buf.push(v.green);
                buf.push(v.blue);
                buf.push(v.alpha);
                &buf[..]
            }
        }
    }

    // Generates a redis command pipeline that adds a value to a
    // device's history.

    pub fn report_new_value_cmd(&mut self, val: &device::Value) -> redis::Cmd {
        let data = [("value", Builder::to_redis(val, &mut self.scratch))];

        redis::Cmd::xadd(&*self.hist_key, "*", &data)
    }

    pub fn report_bounded_new_value_cmd(
        &mut self,
        val: &device::Value,
        mh: usize,
    ) -> redis::Cmd {
        let opts = redis::streams::StreamMaxlen::Approx(mh);
        let data = [("value", Builder::to_redis(val, &mut self.scratch))];

        redis::Cmd::xadd_maxlen(&*self.hist_key, opts, "*", &data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test correct encoding of device::Value::Bool values.

    #[test]
    fn test_bool_encoder() {
        let mut buf = vec![];

        assert_eq!(
            vec![b'B', b'F'],
            Builder::to_redis(&device::Value::Bool(false), &mut buf)
        );
        assert_eq!(
            vec![b'B', b'T'],
            Builder::to_redis(&device::Value::Bool(true), &mut buf)
        );
    }

    const INT_TEST_CASES: &[(i32, &[u8])] = &[
        (0, &[b'I', 0x00, 0x00, 0x00, 0x00]),
        (1, &[b'I', 0x00, 0x00, 0x00, 0x01]),
        (-1, &[b'I', 0xff, 0xff, 0xff, 0xff]),
        (0x7fffffff, &[b'I', 0x7f, 0xff, 0xff, 0xff]),
        (-0x80000000, &[b'I', 0x80, 0x00, 0x00, 0x00]),
        (0x01234567, &[b'I', 0x01, 0x23, 0x45, 0x67]),
    ];

    // Test correct encoding of device::Value::Int values.

    #[test]
    fn test_int_encoder() {
        let mut buf = vec![];

        for (v, rv) in INT_TEST_CASES {
            assert_eq!(
                *rv,
                Builder::to_redis(&device::Value::Int(*v), &mut buf)
            );
        }
    }

    const FLT_TEST_CASES: &[(f64, &[u8])] = &[
        (0.0, &[b'D', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
        (
            -0.0,
            &[b'D', 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        ),
        (1.0, &[b'D', 0x3f, 0xf0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]),
        (
            -1.0,
            &[b'D', 0xbf, 0xf0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00],
        ),
        (
            9007199254740991.0,
            &[b'D', 67, 63, 255, 255, 255, 255, 255, 255],
        ),
        (9007199254740992.0, &[b'D', 67, 64, 0, 0, 0, 0, 0, 0]),
    ];

    // Test correct encoding of device::Value::Flt values.

    #[test]
    fn test_float_encoder() {
        let mut buf = vec![];

        for (v, rv) in FLT_TEST_CASES {
            assert_eq!(
                *rv,
                Builder::to_redis(&device::Value::Flt(*v), &mut buf)
            );
        }
    }

    const COLOR_TEST_CASES: &[((u8, u8, u8, u8), [u8; 5])] = &[
        ((0, 0, 0, 0), [b'C', 0, 0, 0, 0]),
        ((4, 2, 1, 100), [b'C', 4, 2, 1, 100]),
        ((8, 4, 2, 200), [b'C', 8, 4, 2, 200]),
        ((12, 6, 3, 30), [b'C', 12, 6, 3, 30]),
        ((16, 8, 4, 255), [b'C', 16, 8, 4, 255]),
        ((20, 10, 5, 255), [b'C', 20, 10, 5, 255]),
        ((24, 12, 6, 80), [b'C', 24, 12, 6, 80]),
        ((28, 14, 7, 90), [b'C', 28, 14, 7, 90]),
        ((32, 16, 8, 0), [b'C', 32, 16, 8, 0]),
    ];

    #[test]
    fn test_color_encoder() {
        let mut buf = vec![];

        for ((r, g, b, a), rv) in COLOR_TEST_CASES {
            assert_eq!(
                &rv[..],
                Builder::to_redis(
                    &device::Value::Color(palette::LinSrgba::new(
                        *r, *g, *b, *a
                    )),
                    &mut buf
                )
            );
        }
    }

    const STR_TEST_CASES: &[(&str, &[u8])] = &[
        ("", &[b'S', 0u8, 0u8, 0u8, 0u8]),
        ("ABC", &[b'S', 0u8, 0u8, 0u8, 3u8, b'A', b'B', b'C']),
    ];

    // Test correct encoding of device::Value::Str values.

    #[test]
    fn test_string_encoder() {
        let mut buf = vec![];

        for (v, rv) in STR_TEST_CASES {
            assert_eq!(
                *rv,
                Builder::to_redis(&device::Value::Str((*v).into()), &mut buf)
            );
        }
    }

    #[test]
    fn test_report_value_cmd() {
        let mut report = Builder::new("key".into());

        assert_eq!(
            &report
                .report_new_value_cmd(&(true.into()))
                .get_packed_command(),
            b"*5\r
$4\r\nXADD\r
$3\r\nkey\r
$1\r\n*\r
$5\r\nvalue\r
$2\r\nBT\r\n"
        );
        assert_eq!(
            &report
                .report_new_value_cmd(&(0x00010203i32.into()))
                .get_packed_command(),
            b"*5\r
$4\r\nXADD\r
$3\r\nkey\r
$1\r\n*\r
$5\r\nvalue\r
$5\r\nI\x00\x01\x02\x03\r\n"
        );
        assert_eq!(
            &report
                .report_new_value_cmd(&(0x12345678i32.into()))
                .get_packed_command(),
            b"*5\r
$4\r\nXADD\r
$3\r\nkey\r
$1\r\n*\r
$5\r\nvalue\r
$5\r\nI\x12\x34\x56\x78\r\n"
        );
        assert_eq!(
            &report
                .report_new_value_cmd(&(1.0.into()))
                .get_packed_command(),
            b"*5\r
$4\r\nXADD\r
$3\r\nkey\r
$1\r\n*\r
$5\r\nvalue\r
$9\r\nD\x3f\xf0\x00\x00\x00\x00\x00\x00\r\n"
        );
        assert_eq!(
            &report
                .report_new_value_cmd(&("hello".into()))
                .get_packed_command(),
            b"*5\r
$4\r\nXADD\r
$3\r\nkey\r
$1\r\n*\r
$5\r\nvalue\r
$10\r\nS\x00\x00\x00\x05hello\r\n"
        );

        assert_eq!(
            &report
                .report_bounded_new_value_cmd(&(true.into()), 0)
                .get_packed_command(),
            b"*8\r
$4\r\nXADD\r
$3\r\nkey\r
$6\r\nMAXLEN\r
$1\r\n~\r
$1\r\n0\r
$1\r\n*\r
$5\r\nvalue\r
$2\r\nBT\r\n"
        );
        assert_eq!(
            &report
                .report_bounded_new_value_cmd(&(0x00010203i32.into()), 1)
                .get_packed_command(),
            b"*8\r
$4\r\nXADD\r
$3\r\nkey\r
$6\r\nMAXLEN\r
$1\r\n~\r
$1\r\n1\r
$1\r\n*\r
$5\r\nvalue\r
$5\r\nI\x00\x01\x02\x03\r\n"
        );
        assert_eq!(
            &report
                .report_bounded_new_value_cmd(&(0x12345678i32.into()), 2)
                .get_packed_command(),
            b"*8\r
$4\r\nXADD\r
$3\r\nkey\r
$6\r\nMAXLEN\r
$1\r\n~\r
$1\r\n2\r
$1\r\n*\r
$5\r\nvalue\r
$5\r\nI\x12\x34\x56\x78\r\n"
        );
        assert_eq!(
            &report
                .report_bounded_new_value_cmd(&(1.0.into()), 3)
                .get_packed_command(),
            b"*8\r
$4\r\nXADD\r
$3\r\nkey\r
$6\r\nMAXLEN\r
$1\r\n~\r
$1\r\n3\r
$1\r\n*\r
$5\r\nvalue\r
$9\r\nD\x3f\xf0\x00\x00\x00\x00\x00\x00\r\n"
        );
        assert_eq!(
            &report
                .report_bounded_new_value_cmd(&("hello".into()), 4)
                .get_packed_command(),
            b"*8\r
$4\r\nXADD\r
$3\r\nkey\r
$6\r\nMAXLEN\r
$1\r\n~\r
$1\r\n4\r
$1\r\n*\r
$5\r\nvalue\r
$10\r\nS\x00\x00\x00\x05hello\r\n"
        );
    }
}
