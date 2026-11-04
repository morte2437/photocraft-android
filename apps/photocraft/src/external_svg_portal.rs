//! Launch a sandboxed editor through the desktop, exporting only the writable working SVG.
//! Never execute another Flatpak's payload inside PhotoCraft or grant access to its home.

use super::native::OwnedSvg;
use std::collections::HashMap;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};
use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::{Fd, OwnedObjectPath, OwnedValue, Value};

/// Ask the desktop (xdg-desktop-portal OpenURI) to let the user pick an editor for the working
/// file, on a fresh session-bus connection with a 5-second method timeout. Returns once the
/// portal answers; `stop` or a 120-second deadline cancels the wait.
pub(super) fn open(owned: &OwnedSvg, stop: &Receiver<()>) -> Result<(), String> {
    let connection =
        zbus::blocking::connection::Builder::session().map_err(|e| e.to_string())?.method_timeout(Duration::from_secs(5)).build().map_err(|e| e.to_string())?;
    open_on(&connection, owned, stop)
}

/// [`open`] on an existing connection: share a checked writable handle (never a pathname) with
/// `OpenFile`, always asking the user to choose, and wait for the named request's response.
fn open_on(connection: &Connection, owned: &OwnedSvg, stop: &Receiver<()>) -> Result<(), String> {
    use futures_lite::StreamExt;
    // Retain the checked handle for the portal call. Writable document-portal access also
    // supports atomic saves. The watcher keeps using PhotoCraft's original owned pathname.
    let file = owned.share_handle()?;
    let unique = connection.unique_name().ok_or("No desktop connection name")?.as_str().trim_start_matches(':').replace('.', "_");
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let token = format!("photocraft_vector_{}_{}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let path = format!("/org/freedesktop/portal/desktop/request/{unique}/{token}");
    let request = Proxy::new(connection, "org.freedesktop.portal.Desktop", path.as_str(), "org.freedesktop.portal.Request").map_err(|e| e.to_string())?;
    // Subscribe before launching: the portal can finish before OpenFile returns.
    let mut replies = futures_lite::future::block_on(request.inner().receive_signal("Response")).map_err(|e| e.to_string())?;
    let portal = Proxy::new(connection, "org.freedesktop.portal.Desktop", "/org/freedesktop/portal/desktop", "org.freedesktop.portal.OpenURI")
        .map_err(|e| e.to_string())?;
    // SVG's desktop default may be a viewer or browser. Always let the user select an editor.
    let options = HashMap::from([("handle_token", Value::from(token.as_str())), ("writable", Value::from(true)), ("ask", Value::from(true))]);
    let handle: OwnedObjectPath =
        portal.call("OpenFile", &("", Fd::from(&file), options)).map_err(|e| format!("Cannot open the desktop editor chooser: {e}"))?;
    if handle.as_str() != path {
        let returned =
            Proxy::new(connection, "org.freedesktop.portal.Desktop", handle.as_str(), "org.freedesktop.portal.Request").map_err(|e| e.to_string())?;
        let _: Result<(), _> = returned.call("Close", &());
        return Err("The desktop portal does not support named requests".into());
    }
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if !matches!(stop.try_recv(), Err(TryRecvError::Empty)) || Instant::now() >= deadline {
            let _: Result<(), _> = request.call("Close", &());
            return Err("Vector editor selection cancelled or timed out".into());
        }
        let next = futures_lite::future::block_on(futures_lite::future::race(async { Some(replies.next().await) }, async {
            async_io::Timer::after(Duration::from_millis(50)).await;
            None
        }));
        if let Some(reply) = next {
            let reply = reply.ok_or("The desktop editor chooser closed without an answer")?;
            let (code, _): (u32, HashMap<String, OwnedValue>) = reply.body().deserialize().map_err(|e| e.to_string())?;
            return match code {
                0 => Ok(()),
                1 => Err("Vector editor selection cancelled".into()),
                _ => Err("The desktop could not open the vector editor".into()),
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, Read, Seek, Write};
    use std::process::{Child, Command, Stdio};
    use std::sync::mpsc;

    struct Bus(Child);
    impl Drop for Bus {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn bus() -> (Bus, Connection, Connection) {
        let guard = crate::CHILD_SPAWNS.read().unwrap();
        let mut child = Command::new("dbus-daemon").args(["--session", "--nofork", "--print-address=1"]).stdout(Stdio::piped()).spawn().unwrap();
        drop(guard);
        let mut address = String::new();
        std::io::BufReader::new(child.stdout.take().unwrap()).read_line(&mut address).unwrap();
        let server = zbus::blocking::connection::Builder::address(address.trim()).unwrap().name("org.freedesktop.portal.Desktop").unwrap().build().unwrap();
        let client = zbus::blocking::connection::Builder::address(address.trim()).unwrap().method_timeout(Duration::from_secs(2)).build().unwrap();
        (Bus(child), server, client)
    }

    /// A real D-Bus FD transfer carries only the owned SVG, with explicit writable/chooser
    /// options. An immediate response cannot race subscription; writing that FD updates the
    /// original watched path. A rejected chooser never succeeds or launches a fallback.
    #[test]
    fn shares_a_writable_working_copy_and_handles_chooser_responses() {
        for code in [0u32, 1, 2] {
            let (_bus, server, client) = bus();
            let owned = OwnedSvg::create(b"<svg/>").unwrap();
            let messages = zbus::blocking::MessageIterator::from(&server);
            let worker = std::thread::spawn(move || {
                for message in messages {
                    let message = message.unwrap();
                    let header = message.header();
                    if header.member().map(|m| m.as_str()) != Some("OpenFile") {
                        continue;
                    }
                    let (parent, fd, options): (String, zbus::zvariant::OwnedFd, HashMap<String, OwnedValue>) = message.body().deserialize().unwrap();
                    assert!(parent.is_empty());
                    assert!(bool::try_from(options["ask"].try_clone().unwrap()).unwrap());
                    assert!(bool::try_from(options["writable"].try_clone().unwrap()).unwrap());
                    let mut file = std::fs::File::from(std::os::fd::OwnedFd::from(fd));
                    let mut bytes = String::new();
                    file.read_to_string(&mut bytes).unwrap();
                    assert_eq!(bytes, "<svg/>");
                    if code == 0 {
                        file.rewind().unwrap();
                        file.write_all(b"<svg >").unwrap();
                        file.sync_all().unwrap();
                    }
                    let token = String::try_from(options["handle_token"].try_clone().unwrap()).unwrap();
                    let sender = header.sender().unwrap().as_str().trim_start_matches(':').replace('.', "_");
                    let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
                    server.reply(&header, &OwnedObjectPath::try_from(path.as_str()).unwrap()).unwrap();
                    server
                        .emit_signal(
                            header.sender().cloned().map(zbus::names::BusName::from),
                            path.as_str(),
                            "org.freedesktop.portal.Request",
                            "Response",
                            &(code, HashMap::<String, Value<'_>>::new()),
                        )
                        .unwrap();
                    // Keep the owner alive until the client has consumed its response.
                    return server;
                }
                panic!("OpenFile was not called");
            });
            let (_stop, stopped) = mpsc::channel();
            assert_eq!(open_on(&client, &owned, &stopped).is_ok(), code == 0);
            let _server = worker.join().unwrap();
            assert_eq!(std::fs::read(&owned.file).unwrap(), if code == 0 { b"<svg >" } else { b"<svg/>" });
        }
    }

    #[test]
    fn refuses_redirected_or_nonregular_files_before_contacting_the_portal() {
        let (_bus, _server, client) = bus();
        let owned = OwnedSvg::create(b"<svg/>").unwrap();
        let other = tempfile::NamedTempFile::new().unwrap();
        std::fs::remove_file(&owned.file).unwrap();
        std::os::unix::fs::symlink(other.path(), &owned.file).unwrap();
        let (_stop, stopped) = mpsc::channel();
        assert!(open_on(&client, &owned, &stopped).unwrap_err().contains("regular file"));
        std::fs::remove_file(&owned.file).unwrap();
        rustix::fs::mkfifoat(rustix::fs::CWD, &owned.file, rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR).unwrap();
        assert!(open_on(&client, &owned, &stopped).unwrap_err().contains("regular file"));
    }
    #[test]
    fn stopping_sync_closes_an_unanswered_desktop_request() {
        let (_bus, server, client) = bus();
        let owned = OwnedSvg::create(b"<svg/>").unwrap();
        let messages = zbus::blocking::MessageIterator::from(&server);
        let (stop, stopped) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let mut pending_stop = Some(stop);
            for message in messages {
                let message = message.unwrap();
                let header = message.header();
                match header.member().map(|member| member.as_str()) {
                    Some("OpenFile") => {
                        let (_, _, options): (String, zbus::zvariant::OwnedFd, HashMap<String, OwnedValue>) = message.body().deserialize().unwrap();
                        let token = String::try_from(options["handle_token"].try_clone().unwrap()).unwrap();
                        let sender = header.sender().unwrap().as_str().trim_start_matches(':').replace('.', "_");
                        let path = format!("/org/freedesktop/portal/desktop/request/{sender}/{token}");
                        server.reply(&header, &OwnedObjectPath::try_from(path.as_str()).unwrap()).unwrap();
                        pending_stop.take().unwrap().send(()).unwrap();
                    }
                    Some("Close") => {
                        server.reply(&header, &()).unwrap();
                        return;
                    }
                    _ => {}
                }
            }
            panic!("The desktop request was not closed");
        });
        assert!(open_on(&client, &owned, &stopped).unwrap_err().contains("cancelled"));
        worker.join().unwrap();
    }
}
