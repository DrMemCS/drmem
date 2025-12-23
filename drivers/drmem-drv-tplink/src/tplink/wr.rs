use std::io::Write;

// This type allows us to write a TP-Link command into a vector in one
// pass. When it is created, it contains the initial key. As data is
// written, it is "encrypted" and the key is updated.

pub struct CmdWriter<'a> {
    key: u8,
    buf: &'a mut Vec<u8>,
}

impl<'a> CmdWriter<'a> {
    // Creates a new, initialized writer. The parameter is the vector
    // that is to receive the encrypted data.

    pub fn create(b: &'a mut Vec<u8>) -> Self {
        CmdWriter { key: 171u8, buf: b }
    }
}

impl Write for CmdWriter<'_> {
    // This is a mandatory method, but it doesn't do anything.

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }

    // Writes a buffer of data to the vector. As the data is
    // transferred, it is "encrypted". Returns the number of bytes
    // written (which is always the number passed in.)

    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        let sz = b.len();

        for ii in b.iter() {
            self.key ^= *ii;
            self.buf.push(self.key);
        }
        Ok(sz)
    }
}
