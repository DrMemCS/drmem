use std::convert::TryFrom;
use redis::*;

// `Type` defines the primitive types available to devices. Each
// enumeration value wraps a unique, native Rust type.

enum Type {
    Bool(bool),
    Int(i64),
    Flt(f64)
}

// Implement the `ToRedisArgs` trait. This allows us to specify a
// `Type` when writing values to redis so they get encoded correctly.

impl ToRedisArgs for Type {
    fn write_redis_args<W>(&self, out: &mut W)
    where W: ?Sized + RedisWrite,
    {
	match self {
	    Type::Bool(false) => out.write_arg(b"F"),
	    Type::Bool(true) => out.write_arg(b"T"),
	    Type::Int(v) => {
		let mut buf = ['I' as u8; 9];

		buf[1..].copy_from_slice(&v.to_be_bytes());
		out.write_arg(&buf)
	    },
	    Type::Flt(v) => {
		let mut buf = ['D' as u8; 9];

		buf[1..].copy_from_slice(&v.to_be_bytes());
		out.write_arg(&buf)
	    }
	}
    }
}

// Implement the `FromRedisValue` trait. This trait tries to decode a
// `Type` from a string stored in redis.

impl FromRedisValue for Type {
    fn from_redis_value(v: &Value) -> RedisResult<Self>
    {
	let buf: Vec<u8> = from_redis_value(v)?;

	// The buffer has to have at least one character in order to
	// be decoded.

	if buf.len() > 0 {
	    match buf[0] as char {

		// Decode the two boolean values.

		'F' => Ok(Type::Bool(false)),
		'T' => Ok(Type::Bool(true)),

		// Decode an integer value. The length has to be at
		// least 2 characters to allow us to create the [1..]
		// slice without panicking.

		'I' => {
		    if buf.len() > 1 {
			if let Ok(&buf) = <&[u8; 8]>::try_from(&buf[1..]) {
			    return Ok(Type::Int(i64::from_be_bytes(buf)))
			}
		    }
		    Err(RedisError::from((ErrorKind::TypeError,
					  "integer data too short")))
		},

		// Decode a floating point value. The length has to be
		// at least 2 characters to allow us to create the
		// [1..] slice without panicking.

		'D' => {
		    if buf.len() > 1 {
			if let Ok(&buf) = <&[u8; 8]>::try_from(&buf[1..]) {
			    return Ok(Type::Flt(f64::from_be_bytes(buf)))
			}
		    }
		    Err(RedisError::from((ErrorKind::TypeError,
					  "floating point data too short")))
		},

		// Any other character in the tag field is unknown and
		// can't be decoded as a `Type`.

		_ =>
		    Err(RedisError::from((ErrorKind::TypeError, "unknown tag")))
	    }
	} else {
	    Err(RedisError::from((ErrorKind::TypeError, "empty value")))
	}
    }
}
