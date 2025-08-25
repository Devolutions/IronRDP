use expect_test::expect;
use ironrdp_rdpfile::ParseResult;

const RDP_FILE_SAMPLE: &str = "remoteapplicationmode:i:0
server port:i:3389
promptcredentialonce:i:1
full address:s:192.168.56.101
alternate shell:s:|explorer
remoteapplicationname:s:|explorer
alternate full address:s:some.alternateaddress.ninja
username:s:David
ClearTextPassword:s:Devolutions123!
MalformedLine:s
UnknownType:z:10293";

const RDP_FILE_SAMPLE_2: &str = "remoteapplicationmode:i:50
server port:i:4000
full address:s:192.168.56.2";

#[test]
fn parse_file() {
    let ParseResult { mut properties, errors } = ironrdp_rdpfile::parse(RDP_FILE_SAMPLE);

    expect![[r#"
        {
            "ClearTextPassword": Str(
                "Devolutions123!",
            ),
            "alternate full address": Str(
                "some.alternateaddress.ninja",
            ),
            "alternate shell": Str(
                "|explorer",
            ),
            "full address": Str(
                "192.168.56.101",
            ),
            "promptcredentialonce": Int(
                1,
            ),
            "remoteapplicationmode": Int(
                0,
            ),
            "remoteapplicationname": Str(
                "|explorer",
            ),
            "server port": Int(
                3389,
            ),
            "username": Str(
                "David",
            ),
        }
    "#]]
    .assert_debug_eq(&properties);

    expect![[r#"
        [
            Error {
                kind: MalformedLine {
                    line: "MalformedLine:s",
                },
                line: 9,
            },
            Error {
                kind: UnknownType {
                    ty: "z",
                },
                line: 10,
            },
        ]
    "#]]
    .assert_debug_eq(&errors);

    // Verify the `get` operation.
    assert_eq!(properties.get::<bool>("remoteapplicationmode"), Some(false));
    assert_eq!(properties.get::<bool>("promptcredentialonce"), Some(true));
    assert_eq!(properties.get::<bool>("absentproperty"), None);
    assert_eq!(properties.get::<i64>("server port"), Some(3389));
    assert_eq!(properties.get::<&str>("full address"), Some("192.168.56.101"));

    // Merge another file.
    ironrdp_rdpfile::load(&mut properties, RDP_FILE_SAMPLE_2).expect("valid rdp file format");

    expect![[r#"
        {
            "ClearTextPassword": Str(
                "Devolutions123!",
            ),
            "alternate full address": Str(
                "some.alternateaddress.ninja",
            ),
            "alternate shell": Str(
                "|explorer",
            ),
            "full address": Str(
                "192.168.56.2",
            ),
            "promptcredentialonce": Int(
                1,
            ),
            "remoteapplicationmode": Int(
                50,
            ),
            "remoteapplicationname": Str(
                "|explorer",
            ),
            "server port": Int(
                4000,
            ),
            "username": Str(
                "David",
            ),
        }
    "#]]
    .assert_debug_eq(&properties);

    // Anything that is not 0 is considered to be 'true'.
    assert_eq!(properties.get::<bool>("remoteapplicationmode"), Some(true));
}
