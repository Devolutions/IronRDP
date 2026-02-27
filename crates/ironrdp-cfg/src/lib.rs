// QUESTION: consider auto-generating this file based on a reference file?
// https://gist.github.com/awakecoding/838c7fe2ed3a6208e3ca5d8af25363f6

use ironrdp_propertyset::PropertySet;

pub trait PropertySetExt {
    fn full_address(&self) -> Option<&str>;

    fn server_port(&self) -> Option<i64>;

    fn alternate_full_address(&self) -> Option<&str>;

    fn domain(&self) -> Option<&str>;

    fn enable_credssp_support(&self) -> Option<bool>;

    fn compression(&self) -> Option<bool>;

    fn gateway_hostname(&self) -> Option<&str>;

    fn gateway_usage_method(&self) -> Option<i64>;

    fn gateway_credentials_source(&self) -> Option<i64>;

    fn gateway_username(&self) -> Option<&str>;

    fn gateway_password(&self) -> Option<&str>;

    fn desktop_width(&self) -> Option<i64>;

    fn desktop_height(&self) -> Option<i64>;

    fn desktop_scale_factor(&self) -> Option<i64>;

    fn alternate_shell(&self) -> Option<&str>;

    fn shell_working_directory(&self) -> Option<&str>;

    fn redirect_clipboard(&self) -> Option<bool>;

    fn audio_mode(&self) -> Option<i64>;

    fn remote_application_name(&self) -> Option<&str>;

    fn remote_application_program(&self) -> Option<&str>;

    fn kdc_proxy_url(&self) -> Option<&str>;

    fn kdc_proxy_name(&self) -> Option<&str>;

    fn username(&self) -> Option<&str>;

    /// Target RDP server password - use for testing only
    fn clear_text_password(&self) -> Option<&str>;
}

impl PropertySetExt for PropertySet {
    fn full_address(&self) -> Option<&str> {
        self.get::<&str>("full address")
    }

    fn server_port(&self) -> Option<i64> {
        self.get::<i64>("server port")
    }

    fn alternate_full_address(&self) -> Option<&str> {
        self.get::<&str>("alternate full address")
    }

    fn domain(&self) -> Option<&str> {
        self.get::<&str>("domain")
    }

    fn enable_credssp_support(&self) -> Option<bool> {
        self.get::<bool>("enablecredsspsupport")
    }

    fn compression(&self) -> Option<bool> {
        self.get::<bool>("compression")
    }

    fn gateway_hostname(&self) -> Option<&str> {
        self.get::<&str>("gatewayhostname")
    }

    fn gateway_usage_method(&self) -> Option<i64> {
        self.get::<i64>("gatewayusagemethod")
    }

    fn gateway_credentials_source(&self) -> Option<i64> {
        self.get::<i64>("gatewaycredentialssource")
    }

    fn gateway_username(&self) -> Option<&str> {
        self.get::<&str>("gatewayusername")
    }

    fn gateway_password(&self) -> Option<&str> {
        self.get::<&str>("GatewayPassword")
            .or_else(|| self.get::<&str>("gatewaypassword"))
    }

    fn desktop_width(&self) -> Option<i64> {
        self.get::<i64>("desktopwidth")
    }

    fn desktop_height(&self) -> Option<i64> {
        self.get::<i64>("desktopheight")
    }

    fn desktop_scale_factor(&self) -> Option<i64> {
        self.get::<i64>("desktopscalefactor")
    }

    fn alternate_shell(&self) -> Option<&str> {
        self.get::<&str>("alternate shell")
    }

    fn shell_working_directory(&self) -> Option<&str> {
        self.get::<&str>("shell working directory")
    }

    fn redirect_clipboard(&self) -> Option<bool> {
        self.get::<bool>("redirectclipboard")
    }

    fn audio_mode(&self) -> Option<i64> {
        self.get::<i64>("audiomode")
    }

    fn remote_application_name(&self) -> Option<&str> {
        self.get::<&str>("remoteapplicationname")
    }

    fn remote_application_program(&self) -> Option<&str> {
        self.get::<&str>("remoteapplicationprogram")
    }

    fn kdc_proxy_url(&self) -> Option<&str> {
        self.get::<&str>("kdcproxyurl")
            .or_else(|| self.get::<&str>("KDCProxyURL"))
    }

    fn kdc_proxy_name(&self) -> Option<&str> {
        self.get::<&str>("kdcproxyname")
    }

    fn username(&self) -> Option<&str> {
        self.get::<&str>("username")
    }

    fn clear_text_password(&self) -> Option<&str> {
        self.get::<&str>("ClearTextPassword")
    }
}
