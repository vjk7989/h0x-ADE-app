//! Fire-and-forget OS notification. `show` returns immediately and never fails:
//! every error is logged at debug level and dropped.

/// Show a desktop notification; an empty `body` shows the title only. A
/// process about to exit joins the returned thread, or the notice is lost.
pub fn show(title: &str, body: &str) -> Option<std::thread::JoinHandle<()>> {
    let title = title.to_owned();
    let body = body.to_owned();
    let spawned = std::thread::Builder::new()
        .name("os-notice".into())
        .spawn(move || {
            if let Err(err) = platform::show(&title, &body) {
                tracing::debug!("os notification failed: {err}");
            }
        });
    spawned.inspect_err(|err| tracing::debug!("os notification thread failed: {err}")).ok()
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn args_for_osascript(title: &str, body: &str) -> Vec<String> {
    vec![
        "-e".into(),
        "display notification (item 2 of argv) with title (item 1 of argv)".into(),
        title.into(),
        body.into(),
    ]
}

#[cfg_attr(any(windows, target_os = "macos", not(unix)), allow(dead_code))]
fn args_for_notify_send(title: &str, body: &str) -> Vec<String> {
    let mut args = vec!["--app-name=HyperOx".to_owned(), title.to_owned()];
    if !body.is_empty() {
        args.push(body.to_owned());
    }
    args
}

#[cfg(windows)]
mod platform {
    use std::io;
    use std::ptr::{null, null_mut};

    use windows::{
        Data::Xml::Dom::XmlDocument,
        UI::Notifications::{ToastNotification, ToastNotificationManager},
        Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx},
        core::HSTRING,
    };
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
        RegCreateKeyExW, RegSetValueExW,
    };

    const APP_ID: &str = "HyperOx.Desktop";

    fn winrt<T>(result: windows::core::Result<T>) -> io::Result<T> {
        result.map_err(io::Error::other)
    }

    fn wide_null(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Give the toast source a friendly name. Only `DisplayName` is written.
    fn register_display_name() -> io::Result<()> {
        let path = wide_null(&format!(r"Software\Classes\AppUserModelId\{APP_ID}"));
        let mut key: HKEY = null_mut();
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                path.as_ptr(),
                0,
                null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                null(),
                &mut key,
                null_mut(),
            )
        };
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        let value = wide_null("HyperOx");
        let status = unsafe {
            RegSetValueExW(
                key,
                wide_null("DisplayName").as_ptr(),
                0,
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * 2) as u32,
            )
        };
        unsafe {
            RegCloseKey(key);
        }
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        Ok(())
    }

    fn notification_xml(title: &str, body: &str) -> io::Result<XmlDocument> {
        let xml = winrt(XmlDocument::new())?;
        winrt(xml.LoadXml(&HSTRING::from(
            r#"<toast><visual><binding template="ToastGeneric"/></visual><audio silent="true"/></toast>"#,
        )))?;
        let binding = winrt(winrt(xml.GetElementsByTagName(&HSTRING::from("binding")))?.Item(0))?;
        let texts = std::iter::once(title).chain((!body.is_empty()).then_some(body));
        for text in texts {
            let element = winrt(xml.CreateElement(&HSTRING::from("text")))?;
            let node = winrt(xml.CreateTextNode(&HSTRING::from(text)))?;
            winrt(element.AppendChild(&node))?;
            winrt(binding.AppendChild(&element))?;
        }
        Ok(xml)
    }

    /// The toast's XML only: no toast is shown and the registry is not touched.
    #[cfg(test)]
    mod tests {
        use super::*;

        fn texts(xml: &XmlDocument) -> Vec<String> {
            let list = xml.GetElementsByTagName(&HSTRING::from("text")).unwrap();
            (0..list.Length().unwrap()).map(|i| list.Item(i).unwrap().InnerText().unwrap().to_string()).collect()
        }

        #[test]
        fn the_toast_text_is_escaped_and_reads_back_literally() {
            winrt(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok() }).unwrap();
            let title = "HyperOx has been launched";
            let body = r#"D:\a <b> & "c" 'd' </text><text>x</text>"#;
            let xml = notification_xml(title, body).unwrap();
            let out = xml.GetXml().unwrap().to_string();
            // Markup in the folder stays text: escaped, never an element of its own.
            assert!(out.contains("&lt;b"), "{out}");
            assert!(out.contains("&amp;"), "{out}");
            assert!(!out.contains("<b>"), "{out}");
            assert!(!out.contains("<text>x</text>"), "{out}");
            assert!(out.contains(r#"<binding template="ToastGeneric">"#), "{out}");
            assert!(out.contains(r#"<audio silent="true"/>"#) || out.contains(r#"<audio silent="true" />"#), "{out}");
            assert_eq!(texts(&xml), [title, body]);
            // Parsed again, as the toast platform does, it is the same text.
            let again = XmlDocument::new().unwrap();
            again.LoadXml(&HSTRING::from(out.as_str())).unwrap();
            assert_eq!(texts(&again), [title, body]);
        }

        #[test]
        fn an_empty_body_gives_a_title_only_toast() {
            winrt(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok() }).unwrap();
            assert_eq!(texts(&notification_xml("HyperOx has been launched", "").unwrap()), ["HyperOx has been launched"]);
            // A unicode or UNC folder is kept as it is.
            let body = "\\\\server\\share\\caf\u{e9} \u{65e5}\u{672c}";
            assert_eq!(texts(&notification_xml("t", body).unwrap()), ["t", body]);
        }
    }

    pub fn show(title: &str, body: &str) -> io::Result<()> {
        // Never uninitialized: the windows crate caches activation factories across calls.
        winrt(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED).ok() })?;
        register_display_name()?;
        let xml = notification_xml(title, body)?;
        let notification = winrt(ToastNotification::CreateToastNotification(&xml))?;
        let notifier = winrt(ToastNotificationManager::CreateToastNotifierWithId(
            &HSTRING::from(APP_ID),
        ))?;
        winrt(notifier.Show(&notification))
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::io;
    use std::process::{Command, Stdio};

    pub fn show(title: &str, body: &str) -> io::Result<()> {
        Command::new("/usr/bin/osascript")
            .args(super::args_for_osascript(title, body))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|_| ())
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod platform {
    use std::io;
    use std::process::{Command, Stdio};

    pub fn show(title: &str, body: &str) -> io::Result<()> {
        match Command::new("notify-send")
            .args(super::args_for_notify_send(title, body))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other.map(|_| ()),
        }
    }
}

#[cfg(not(any(windows, unix)))]
mod platform {
    pub fn show(_title: &str, _body: &str) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{args_for_notify_send, args_for_osascript};

    const ODD: &str = "/tmp/it's \"quoted\" & <x> $(rm -rf ~) `id` \u{65e5}";

    #[test]
    fn osascript_gets_the_title_and_body_as_arguments_never_in_its_script() {
        let args = args_for_osascript("HyperOx has been launched", ODD);
        assert_eq!(args.len(), 4);
        assert_eq!(args[0], "-e");
        assert_eq!(args[1], "display notification (item 2 of argv) with title (item 1 of argv)");
        assert_eq!(args[2], "HyperOx has been launched");
        assert_eq!(args[3], ODD);
        assert!(!args[1].contains("HyperOx") && !args[1].contains(ODD));
        // An empty body is still `item 2`, so the script never fails on a missing item.
        let args = args_for_osascript("t", "");
        assert_eq!(args.len(), 4);
        assert_eq!(args[3], "");
        // A title that looks like AppleScript stays a value.
        let args = args_for_osascript("\" & (do shell script \"id\") & \"", "b");
        assert_eq!(args[1], "display notification (item 2 of argv) with title (item 1 of argv)");
        assert_eq!(args[2], "\" & (do shell script \"id\") & \"");
    }

    #[test]
    fn notify_send_names_the_app_and_leaves_out_an_empty_body() {
        assert_eq!(args_for_notify_send("HyperOx has been launched", ""), ["--app-name=HyperOx", "HyperOx has been launched"]);
        assert_eq!(args_for_notify_send("HyperOx has been launched", ODD), ["--app-name=HyperOx", "HyperOx has been launched", ODD]);
        // A body that looks like an option is still the body, after the title.
        let args = args_for_notify_send("t", "--urgency=critical");
        assert_eq!(args, ["--app-name=HyperOx", "t", "--urgency=critical"]);
    }
}
