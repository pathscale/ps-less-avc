use less_avc::sei::{SupplementalEnhancementInformation, UserDataUnregistered};

const UUID: [u8; 16] = [
    0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xab, 0xac, 0xad,
    0xae, 0xaf,
];

fn assert_encoded_payload(user_data_len: usize, expected_header: &[u8]) {
    let user_data: Vec<u8> = (0..user_data_len).map(|byte| byte as u8).collect();
    let sei = SupplementalEnhancementInformation::UserDataUnregistered(
        UserDataUnregistered::new(UUID, user_data.clone()),
    );

    let mut expected = Vec::with_capacity(expected_header.len() + UUID.len() + user_data.len() + 1);
    expected.extend_from_slice(expected_header);
    expected.extend_from_slice(&UUID);
    expected.extend_from_slice(&user_data);
    expected.push(0x80);

    assert_eq!(sei.to_rbsp().data, expected);
}

#[test]
fn encodes_sei_payload_size_boundaries() {
    // Aggregate payload sizes include the 16-byte UUID.
    assert_encoded_payload(238, &[0x05, 0xfe]);
    assert_encoded_payload(239, &[0x05, 0xff, 0x00]);
    assert_encoded_payload(240, &[0x05, 0xff, 0x01]);
    assert_encoded_payload(494, &[0x05, 0xff, 0xff, 0x00]);
}
