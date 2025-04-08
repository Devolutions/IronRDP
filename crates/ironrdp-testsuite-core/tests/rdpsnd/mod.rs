use std::borrow::Cow;

use ironrdp_rdpsnd::pdu;
use ironrdp_testsuite_core::encode_decode_test;

encode_decode_test! {
    server_format: pdu::ServerAudioOutputPdu::AudioFormat(pdu::ServerAudioFormatPdu {
        version: pdu::Version::V5,
        formats: vec![
            pdu::AudioFormat {
                format: pdu::WaveFormat::PCM,
                n_channels: 2,
                n_samples_per_sec: 22050,
                n_avg_bytes_per_sec: 88200,
                n_block_align: 4,
                bits_per_sample: 16,
                data: None,
            },
            pdu::AudioFormat {
                format: pdu::WaveFormat::ALAW,
                n_channels: 2,
                n_samples_per_sec: 22050,
                n_avg_bytes_per_sec: 44100,
                n_block_align: 2,
                bits_per_sample: 8,
                data: None,
            },
            pdu::AudioFormat {
                format: pdu::WaveFormat::MULAW,
                n_channels: 2,
                n_samples_per_sec: 22050,
                n_avg_bytes_per_sec: 44100,
                n_block_align: 2,
                bits_per_sample: 8,
                data: None,
            },
            pdu::AudioFormat {
                format: pdu::WaveFormat::ADPCM,
                n_channels: 2,
                n_samples_per_sec: 22050,
                n_avg_bytes_per_sec: 22311,
                n_block_align: 1024,
                bits_per_sample: 4,
                data: Some(vec![
                    0xf4, 0x03, 0x07, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0xff, 0x00, 0x00, 0x00, 0x00, 0xc0, 0x00, 0x40, 0x00, 0xf0, 0x00, 0x00,
                    0x00, 0xcc, 0x01, 0x30, 0xff, 0x88, 0x01, 0x18, 0xff,
                ])
            },
            pdu::AudioFormat {
                format: pdu::WaveFormat::DVI_ADPCM,
                n_channels: 2,
                n_samples_per_sec: 22050,
                n_avg_bytes_per_sec: 22201,
                n_block_align: 1024,
                bits_per_sample: 4,
                data: Some(vec![
                    0xf9,  0x03,
                ])
            },
        ],
    }),
    [
        0x07,  0x00,  0x90,  0x00,  0x00,  0x00,  0x00,  0x00,  0x00,  0x00,  0x00,  0x00,  0x00,  0x00,  0x00,  0x00,
        0x00,  0x00,  0x05,  0x00,  0x00,  0x05,  0x00,  0x00,  0x01,  0x00,  0x02,  0x00,  0x22,  0x56,  0x00,  0x00,
        0x88,  0x58,  0x01,  0x00,  0x04,  0x00,  0x10,  0x00,  0x00,  0x00,  0x06,  0x00,  0x02,  0x00,  0x22,  0x56,
        0x00,  0x00,  0x44,  0xac,  0x00,  0x00,  0x02,  0x00,  0x08,  0x00,  0x00,  0x00,  0x07,  0x00,  0x02,  0x00,
        0x22,  0x56,  0x00,  0x00,  0x44,  0xac,  0x00,  0x00,  0x02,  0x00,  0x08,  0x00,  0x00,  0x00,  0x02,  0x00,
        0x02,  0x00,  0x22,  0x56,  0x00,  0x00,  0x27,  0x57,  0x00,  0x00,  0x00,  0x04,  0x04,  0x00,  0x20,  0x00,
        0xf4,  0x03,  0x07,  0x00,  0x00,  0x01,  0x00,  0x00,  0x00,  0x02,  0x00,  0xff,  0x00,  0x00,  0x00,  0x00,
        0xc0,  0x00,  0x40,  0x00,  0xf0,  0x00,  0x00,  0x00,  0xcc,  0x01,  0x30,  0xff,  0x88,  0x01,  0x18,  0xff,
        0x11,  0x00,  0x02,  0x00,  0x22,  0x56,  0x00,  0x00,  0xb9,  0x56,  0x00,  0x00,  0x00,  0x04,  0x04,  0x00,
        0x02,  0x00,  0xf9,  0x03,
    ];
    client_format: pdu::ClientAudioOutputPdu::AudioFormat(pdu::ClientAudioFormatPdu {
        version: pdu::Version::V5,
        flags: pdu::AudioFormatFlags::ALIVE | pdu::AudioFormatFlags::VOLUME,
        volume_left: 0xFFFF,
        volume_right: 0xFFFF,
        pitch: 0xF9F700,
        dgram_port: 0,
        formats: vec![
            pdu::AudioFormat {
                format: pdu::WaveFormat::PCM,
                n_channels: 2,
                n_samples_per_sec: 22050,
                n_avg_bytes_per_sec: 88200,
                n_block_align: 4,
                bits_per_sample: 16,
                data: None,
            },
            pdu::AudioFormat {
                format: pdu::WaveFormat::ALAW,
                n_channels: 2,
                n_samples_per_sec: 22050,
                n_avg_bytes_per_sec: 44100,
                n_block_align: 2,
                bits_per_sample: 8,
                data: None,
            },
            pdu::AudioFormat {
                format: pdu::WaveFormat::MULAW,
                n_channels: 2,
                n_samples_per_sec: 22050,
                n_avg_bytes_per_sec: 44100,
                n_block_align: 2,
                bits_per_sample: 8,
                data: None,
            },
            pdu::AudioFormat {
                format: pdu::WaveFormat::ADPCM,
                n_channels: 2,
                n_samples_per_sec: 22050,
                n_avg_bytes_per_sec: 22311,
                n_block_align: 1024,
                bits_per_sample: 4,
                data: Some(vec![
                    0xf4, 0x03, 0x07, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0xff, 0x00, 0x00, 0x00, 0x00, 0xc0, 0x00, 0x40, 0x00, 0xf0, 0x00, 0x00,
                    0x00, 0xcc, 0x01, 0x30, 0xff, 0x88, 0x01, 0x18, 0xff,
                ])
            },
            pdu::AudioFormat {
                format: pdu::WaveFormat::DVI_ADPCM,
                n_channels: 2,
                n_samples_per_sec: 22050,
                n_avg_bytes_per_sec: 22201,
                n_block_align: 1024,
                bits_per_sample: 4,
                data: Some(vec![
                    0xf9,  0x03,
                ])
            },
        ],
    }),
    [
        0x07, 0x00, 0x90, 0x00, 0x03, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0x00, 0xf7, 0xf9, 0x00,
        0x00, 0x00, 0x05, 0x00, 0x00, 0x05, 0x00, 0x00, 0x01, 0x00, 0x02, 0x00, 0x22, 0x56, 0x00, 0x00,
        0x88, 0x58, 0x01, 0x00, 0x04, 0x00, 0x10, 0x00, 0x00, 0x00, 0x06, 0x00, 0x02, 0x00, 0x22, 0x56,
        0x00, 0x00, 0x44, 0xac, 0x00, 0x00, 0x02, 0x00, 0x08, 0x00, 0x00, 0x00, 0x07, 0x00, 0x02, 0x00,
        0x22, 0x56, 0x00, 0x00, 0x44, 0xac, 0x00, 0x00, 0x02, 0x00, 0x08, 0x00, 0x00, 0x00, 0x02, 0x00,
        0x02, 0x00, 0x22, 0x56, 0x00, 0x00, 0x27, 0x57, 0x00, 0x00, 0x00, 0x04, 0x04, 0x00, 0x20, 0x00,
        0xf4, 0x03, 0x07, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02, 0x00, 0xff, 0x00, 0x00, 0x00, 0x00,
        0xc0, 0x00, 0x40, 0x00, 0xf0, 0x00, 0x00, 0x00, 0xcc, 0x01, 0x30, 0xff, 0x88, 0x01, 0x18, 0xff,
        0x11, 0x00, 0x02, 0x00, 0x22, 0x56, 0x00, 0x00, 0xb9, 0x56, 0x00, 0x00, 0x00, 0x04, 0x04, 0x00,
        0x02, 0x00, 0xf9, 0x03,
    ];
    training: pdu::ServerAudioOutputPdu::Training(pdu::TrainingPdu {
        timestamp: 0x89da,
        data: vec![0x42],
    }),
    [
        0x06, 0x00, 0x05, 0x00, 0xda, 0x89, 0x09, 0x00, 0x42
    ];
    training_confirm: pdu::ClientAudioOutputPdu::TrainingConfirm(pdu::TrainingConfirmPdu {
        timestamp: 0x89da,
        pack_size: 0x400,
    }),
    [
        0x06, 0x00, 0x04, 0x00, 0xda, 0x89, 0x00, 0x04,
    ];
    wave: pdu::ServerAudioOutputPdu::Wave(pdu::WavePdu {
        timestamp: 0xadd7,
        format_no: 0xf,
        block_no: 8,
        data: Cow::Borrowed(&[0x1, 0x2, 0x3, 0x4, 0x5, 0x6, 0x7, 0x8]),
    }),
    [
        // WaveInfo
        0x02, 0x00, 0x10, 0x00, 0xd7, 0xad, 0x0f, 0x00, 0x08, 0x00, 0x00, 0x00, 0x1, 0x2, 0x3, 0x4,
        // Wave
        0x0, 0x0, 0x0, 0x0, 0x5, 0x6, 0x7, 0x8,
    ];
    wave_confirm: pdu::ClientAudioOutputPdu::WaveConfirm(pdu::WaveConfirmPdu {
        timestamp: 0x5ab7,
        block_no: 8
    }),
    [
        0x05, 0x00, 0x04, 0x00, 0xb7, 0x5a, 0x08, 0x00,
    ];
    wave2: pdu::ServerAudioOutputPdu::Wave2(pdu::Wave2Pdu {
        timestamp: 0xa116,
        audio_timestamp: 0xdacb8c2,
        format_no: 0x3,
        block_no: 2,
        data: Cow::Borrowed(&[0x1, 0x2, 0x3, 0x4, 0x5, 0x6, 0x7, 0x8]),
    }),
    [
        0x0D, 0x00, 0x14, 0x00, 0x16, 0xA1, 0x03, 0x00, 0x02, 0x00, 0x00, 0x00, 0xC2, 0xB8, 0xAC, 0x0D, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08,
    ];
}
