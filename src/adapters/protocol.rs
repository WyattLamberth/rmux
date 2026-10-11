use std::io::{self, Read, Write};

pub const MAX_FRAME: usize = 262_144;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientMsg {
    Hello { rows: u16, cols: u16 },
    Write(Vec<u8>),
    Action(u8),
    Resize { rows: u16, cols: u16 },
    Detach,
    Quit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServerMsg {
    Hello { key: [u8; 32], udp_port: u16 },
    Frame { epoch: u64, bytes: Vec<u8> },
    Bye,
}

pub fn write_client(mut writer: impl Write, msg: &ClientMsg) -> io::Result<()> {
    write_raw(&mut writer, &encode_client(msg)?)
}

pub fn write_server(mut writer: impl Write, msg: &ServerMsg) -> io::Result<()> {
    write_raw(&mut writer, &encode_server(msg)?)
}

pub fn read_client(reader: &mut impl Read) -> io::Result<ClientMsg> {
    decode_client(&read_raw(reader)?)
}

pub fn read_server(reader: &mut impl Read) -> io::Result<ServerMsg> {
    decode_server(&read_raw(reader)?)
}

fn write_raw(writer: &mut impl Write, body: &[u8]) -> io::Result<()> {
    if body.len() > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    writer.write_all(&(body.len() as u32).to_be_bytes())?;
    writer.write_all(body)?;
    Ok(())
}

fn read_raw(reader: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut len = [0; 4];
    reader.read_exact(&mut len)?;
    let len = u32::from_be_bytes(len) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame length"));
    }
    let mut body = vec![0; len];
    reader.read_exact(&mut body)?;
    Ok(body)
}

fn encode_client(msg: &ClientMsg) -> io::Result<Vec<u8>> {
    let mut body = Vec::new();
    match msg {
        ClientMsg::Hello { rows, cols } => {
            body.push(1);
            body.extend(rows.to_be_bytes());
            body.extend(cols.to_be_bytes());
        }
        ClientMsg::Write(bytes) => {
            if bytes.len() > 65_536 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "write too large",
                ));
            }
            body.push(2);
            body.extend((bytes.len() as u32).to_be_bytes());
            body.extend(bytes);
        }
        ClientMsg::Action(code) => {
            body.push(3);
            body.push(*code);
        }
        ClientMsg::Resize { rows, cols } => {
            body.push(4);
            body.extend(rows.to_be_bytes());
            body.extend(cols.to_be_bytes());
        }
        ClientMsg::Detach => body.push(5),
        ClientMsg::Quit => body.push(6),
    }
    Ok(body)
}

fn decode_client(body: &[u8]) -> io::Result<ClientMsg> {
    match body.first().copied() {
        Some(1) if body.len() == 5 => Ok(ClientMsg::Hello {
            rows: u16::from_be_bytes([body[1], body[2]]),
            cols: u16::from_be_bytes([body[3], body[4]]),
        }),
        Some(2) if body.len() >= 5 => {
            let len = u32::from_be_bytes([body[1], body[2], body[3], body[4]]) as usize;
            if body.len() != 5 + len || len > 65_536 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "write length"));
            }
            Ok(ClientMsg::Write(body[5..].to_vec()))
        }
        Some(3) if body.len() == 2 => Ok(ClientMsg::Action(body[1])),
        Some(4) if body.len() == 5 => Ok(ClientMsg::Resize {
            rows: u16::from_be_bytes([body[1], body[2]]),
            cols: u16::from_be_bytes([body[3], body[4]]),
        }),
        Some(5) if body.len() == 1 => Ok(ClientMsg::Detach),
        Some(6) if body.len() == 1 => Ok(ClientMsg::Quit),
        _ => Err(io::Error::new(io::ErrorKind::InvalidData, "client message")),
    }
}

fn encode_server(msg: &ServerMsg) -> io::Result<Vec<u8>> {
    let mut body = Vec::new();
    match msg {
        ServerMsg::Hello { key, udp_port } => {
            body.push(1);
            body.extend(key);
            body.extend(udp_port.to_be_bytes());
        }
        ServerMsg::Frame { epoch, bytes } => {
            if bytes.len() > MAX_FRAME - 9 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "frame too large",
                ));
            }
            body.push(2);
            body.extend(epoch.to_be_bytes());
            body.extend(bytes);
        }
        ServerMsg::Bye => body.push(3),
    }
    Ok(body)
}

fn decode_server(body: &[u8]) -> io::Result<ServerMsg> {
    match body.first().copied() {
        Some(1) if body.len() == 35 => {
            let mut key = [0; 32];
            key.copy_from_slice(&body[1..33]);
            Ok(ServerMsg::Hello {
                key,
                udp_port: u16::from_be_bytes([body[33], body[34]]),
            })
        }
        Some(2) if body.len() >= 9 => Ok(ServerMsg::Frame {
            epoch: u64::from_be_bytes(body[1..9].try_into().unwrap()),
            bytes: body[9..].to_vec(),
        }),
        Some(3) if body.len() == 1 => Ok(ServerMsg::Bye),
        _ => Err(io::Error::new(io::ErrorKind::InvalidData, "server message")),
    }
}

pub fn action_code(action: &super::config::Action) -> Option<u8> {
    Some(match action {
        super::config::Action::SplitColumns => 1,
        super::config::Action::SplitRows => 2,
        super::config::Action::FocusNext => 3,
        super::config::Action::FocusPrevious => 4,
        super::config::Action::Zoom => 5,
        super::config::Action::ScrollUp => 6,
        super::config::Action::ScrollDown => 7,
        super::config::Action::CloseFocused => 8,
        super::config::Action::Quit => 9,
        super::config::Action::Help => 10,
        super::config::Action::SendPrefix => 11,
        super::config::Action::Cancel => 12,
        super::config::Action::Detach => 13,
        super::config::Action::RunShell(_) => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_and_server_messages_roundtrip() {
        let messages = [
            ClientMsg::Hello { rows: 24, cols: 80 },
            ClientMsg::Write(b"ls\r".to_vec()),
            ClientMsg::Action(13),
            ClientMsg::Resize {
                rows: 40,
                cols: 120,
            },
            ClientMsg::Detach,
            ClientMsg::Quit,
        ];
        for msg in messages {
            let mut encoded = Vec::new();
            write_client(&mut encoded, &msg).unwrap();
            assert_eq!(read_client(&mut encoded.as_slice()).unwrap(), msg);
        }
        let server = ServerMsg::Frame {
            epoch: 4,
            bytes: b"screen".to_vec(),
        };
        let mut encoded = Vec::new();
        write_server(&mut encoded, &server).unwrap();
        assert_eq!(read_server(&mut encoded.as_slice()).unwrap(), server);
    }

    #[test]
    fn oversized_length_is_rejected() {
        let mut encoded = Vec::new();
        encoded.extend(2_000_000u32.to_be_bytes());
        assert!(read_client(&mut encoded.as_slice()).is_err());
    }
}
