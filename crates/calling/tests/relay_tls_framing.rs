//! RFC 6062 / RFC 8656 §12.5 stream framing for the TURN-over-TLS shim: STUN messages are
//! self-delimiting, ChannelData is padded to 4 bytes on a stream.

use calling::relay::tls::{StreamDeframer, frame_for_stream};

/// A STUN Binding request with `body` bytes of attributes (a multiple of 4).
fn stun(body: usize, tid: u8) -> Vec<u8> {
    let mut m = vec![0x00, 0x01];
    m.extend_from_slice(&(body as u16).to_be_bytes());
    m.extend_from_slice(&[0x21, 0x12, 0xA4, 0x42]);
    m.extend_from_slice(&[tid; 12]);
    m.extend(std::iter::repeat_n(0xAB, body));
    m
}

/// ChannelData on channel 0x4001 carrying `len` bytes (unpadded, as on UDP).
fn channel_data(len: usize) -> Vec<u8> {
    let mut m = vec![0x40, 0x01];
    m.extend_from_slice(&(len as u16).to_be_bytes());
    m.extend((0..len).map(|i| i as u8));
    m
}

fn drain(d: &mut StreamDeframer) -> Vec<Vec<u8>> {
    let mut out = vec![];
    while let Some(f) = d.next_frame().unwrap() {
        out.push(f);
    }
    out
}

#[test]
fn stun_goes_on_the_stream_unchanged() {
    let m = stun(8, 1);
    assert_eq!(frame_for_stream(&m).unwrap(), m);
}

#[test]
fn channel_data_is_padded_to_four_bytes_on_the_stream() {
    let framed = frame_for_stream(&channel_data(5)).unwrap();
    assert_eq!(framed.len(), 12);
    assert_eq!(&framed[..9], &channel_data(5)[..]);
    assert_eq!(&framed[9..], &[0, 0, 0]);
    // Already a multiple of four: no padding.
    assert_eq!(frame_for_stream(&channel_data(8)).unwrap().len(), 12);
}

#[test]
fn padding_a_datagram_already_carries_is_not_doubled() {
    let mut d = channel_data(5);
    d.extend_from_slice(&[0, 0, 0]);
    assert_eq!(frame_for_stream(&d).unwrap().len(), 12);
}

#[test]
fn malformed_datagrams_are_not_framed() {
    assert!(frame_for_stream(&[]).is_none());
    // Neither STUN nor ChannelData (first two bits 10).
    assert!(frame_for_stream(&[0x80, 0, 0, 0]).is_none());
    // STUN shorter than its header says.
    let mut short = stun(8, 1);
    short.truncate(24);
    assert!(frame_for_stream(&short).is_none());
    // ChannelData shorter than its length field says.
    assert!(frame_for_stream(&channel_data(5)[..6]).is_none());
}

#[test]
fn a_frame_split_across_reads_comes_out_whole() {
    let m = stun(12, 2);
    let mut d = StreamDeframer::default();
    d.push(&m[..3]);
    assert!(drain(&mut d).is_empty());
    d.push(&m[3..21]);
    assert!(drain(&mut d).is_empty());
    d.push(&m[21..]);
    assert_eq!(drain(&mut d), vec![m]);
}

#[test]
fn several_frames_in_one_read_come_out_in_order_without_padding() {
    let a = stun(4, 3);
    let b = channel_data(5);
    let c = stun(0, 4);
    let mut bytes = a.clone();
    bytes.extend(frame_for_stream(&b).unwrap());
    bytes.extend(&c);
    let mut d = StreamDeframer::default();
    d.push(&bytes);
    assert_eq!(drain(&mut d), vec![a, b, c]);
}

#[test]
fn padding_split_from_its_channel_data_is_waited_for() {
    let framed = frame_for_stream(&channel_data(5)).unwrap();
    let mut d = StreamDeframer::default();
    d.push(&framed[..9]);
    assert!(drain(&mut d).is_empty(), "the padding is part of the frame");
    d.push(&framed[9..]);
    assert_eq!(drain(&mut d), vec![channel_data(5)]);
}

#[test]
fn a_desynchronised_stream_is_an_error() {
    let mut d = StreamDeframer::default();
    d.push(&[0x80, 0x00, 0x00, 0x04, 1, 2, 3, 4]);
    assert!(d.next_frame().is_err());
    // STUN without the magic cookie.
    let mut m = stun(0, 5);
    m[4] = 0;
    let mut d = StreamDeframer::default();
    d.push(&m);
    assert!(d.next_frame().is_err());
}
