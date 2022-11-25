// TODO: this code should probably get swapped out with something like socket2..

use std::mem::size_of;
use std::mem::size_of_val;
use uuid::Uuid;
use windows::core::GUID;
use windows::core::PWSTR;
use windows::Win32::Devices::Bluetooth::*;
use windows::Win32::Networking::WinSock::*;
use windows::Win32::System::Threading::GetCurrentProcessId;

pub fn platform_init() -> anyhow::Result<()> {
    let mut wsa_data = WSADATA::default();
    let res = unsafe { WSAStartup(0x0202, &mut wsa_data) };
    if res != 0 {
        anyhow::bail!("WSAStartup() failed: {}", res);
    }
    Ok(())
}

pub fn platform_teardown() -> anyhow::Result<()> {
    let res = unsafe { WSACleanup() };
    if res != 0 {
        anyhow::bail!("WSACleanup() failed: {:?}", wsa_get_last_error());
    }
    Ok(())
}

pub struct WinBtSockListener {
    s: SOCKET,
    sa: SOCKADDR_BTH,
}

impl WinBtSockListener {
    pub fn bind() -> anyhow::Result<WinBtSockListener> {
        let s = unsafe { socket(AF_BTH.into(), SOCK_STREAM.into(), BTHPROTO_RFCOMM as i32) };
        if s == INVALID_SOCKET {
            anyhow::bail!("failed to create socket");
        }

        let mut sa_size = size_of::<SOCKADDR_BTH>() as i32;
        let mut sa = SOCKADDR_BTH {
            addressFamily: AF_BTH,
            btAddr: 0,
            serviceClassId: GUID::default(),
            port: !0,
        };

        let res = unsafe { bind(s, &sa as *const _ as *const SOCKADDR, sa_size) };
        if res != 0 {
            close_socket(s)?;
            anyhow::bail!("failed to bind socket: {:?}", wsa_get_last_error());
        }

        let res = unsafe { listen(s, 2) };
        if res != 0 {
            close_socket(s)?;
            anyhow::bail!("failed to listen socket: {:?}", wsa_get_last_error());
        }

        let res = unsafe { getsockname(s, &mut sa as *mut _ as *mut SOCKADDR, &mut sa_size) };
        if res != 0 {
            close_socket(s)?;
            anyhow::bail!("failed to getsockname socket: {:?}", wsa_get_last_error());
        }

        Ok(WinBtSockListener { s, sa })
    }

    pub fn rfcomm_port(&self) -> u32 {
        self.sa.port
    }

    pub fn register_service(&mut self, name: &str, service_id: Uuid) -> anyhow::Result<()> {
        let mut name = name.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
        let mut class_id = GUID::from_u128(service_id.as_u128());

        let mut sockinfo = CSADDR_INFO {
            LocalAddr: SOCKET_ADDRESS {
                lpSockaddr: &mut self.sa as *mut _ as *mut SOCKADDR,
                iSockaddrLength: size_of_val(&self.sa) as i32,
            },
            iSocketType: SOCK_STREAM.into(),
            iProtocol: size_of_val(&self.sa) as i32,
            ..CSADDR_INFO::default()
        };

        let qs = WSAQUERYSETW {
            dwSize: size_of::<WSAQUERYSETW>() as u32,
            lpszServiceInstanceName: PWSTR::from_raw(name.as_mut_ptr()),
            lpServiceClassId: &mut class_id,
            dwNameSpace: NS_BTH,
            dwNumberOfCsAddrs: 1,
            lpcsaBuffer: &mut sockinfo,
            ..WSAQUERYSETW::default()
        };

        // Windows will automatically deregister our btsdp service once our
        // process is no longer running
        //
        // We can't do it ourselves since we don't get a handle to our sdp
        // registration
        let ret = unsafe { WSASetServiceW(&qs, RNRSERVICE_REGISTER, 0) };
        if ret != 0 {
            anyhow::bail!("WSASetService failed: {:?}", wsa_get_last_error())
        }

        Ok(())
    }

    pub fn accept(&mut self) -> anyhow::Result<WinBtSockStream> {
        let mut sa = SOCKADDR_BTH::default();
        let sa_ptr = &mut sa as *mut _ as *mut SOCKADDR;
        let sa_len_ptr = &mut (size_of::<SOCKADDR_BTH>() as i32) as *mut i32;

        let s = unsafe { accept(self.s, Some(sa_ptr), Some(sa_len_ptr)) };
        if s == INVALID_SOCKET {
            anyhow::bail!("failed to accept socket: {:?}", wsa_get_last_error())
        }

        Ok(WinBtSockStream { s, sa })
    }
}

impl Drop for WinBtSockListener {
    fn drop(&mut self) {
        close_socket(self.s).expect("failed to close socket on drop")
    }
}

pub struct WinBtSockStream {
    s: SOCKET,
    sa: SOCKADDR_BTH,
}

impl WinBtSockStream {
    pub fn nap(&self) -> u16 {
        (self.sa.btAddr >> 32) as u16
    }

    pub fn sap(&self) -> u32 {
        self.sa.btAddr as u32
    }

    pub fn port(&self) -> u32 {
        self.sa.port
    }

    pub fn try_clone(&self) -> anyhow::Result<Self> {
        let mut info = WSAPROTOCOL_INFOW::default();
        let res = unsafe { WSADuplicateSocketW(self.s, GetCurrentProcessId(), &mut info) };
        if res != 0 {
            anyhow::bail!("failed to duplicate socket: {:?}", wsa_get_last_error())
        }
        let s = unsafe {
            WSASocketW(
                info.iAddressFamily,
                info.iSocketType,
                info.iProtocol,
                Some(&info),
                0,
                0,
            )
        };

        if s == INVALID_SOCKET {
            anyhow::bail!("failed to create socket");
        }

        Ok(WinBtSockStream { s, sa: self.sa })
    }
}

impl std::io::Read for WinBtSockStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let res = unsafe { recv(self.s, buf, SEND_RECV_FLAGS::default()) };
        match res {
            SOCKET_ERROR => {
                let error = wsa_get_last_error();

                if error == WSAESHUTDOWN {
                    Ok(0)
                } else {
                    Err(std::io::Error::from_raw_os_error(error.0))
                }
            }
            _ => Ok(res as usize),
        }
    }
}

impl std::io::Write for WinBtSockStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let res = unsafe { send(self.s, buf, SEND_RECV_FLAGS::default()) };
        match res {
            SOCKET_ERROR => {
                let error = wsa_get_last_error();

                if error == WSAESHUTDOWN {
                    Ok(0)
                } else {
                    Err(std::io::Error::from_raw_os_error(error.0))
                }
            }
            _ => Ok(res as usize),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn close_socket(socket: SOCKET) -> anyhow::Result<()> {
    let res = unsafe { closesocket(socket) };
    if res != 0 {
        anyhow::bail!("failed to close socket: {:?}", wsa_get_last_error())
    }
    Ok(())
}

fn wsa_get_last_error() -> WSA_ERROR {
    unsafe { WSAGetLastError() }
}
