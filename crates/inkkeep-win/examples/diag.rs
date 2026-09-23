//! 診斷 virtual 模式的鍵盤對映。
#![cfg(windows)]

use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardLayout, MapVirtualKeyW, VkKeyScanExW, MAPVK_VK_TO_VSC,
};

fn main() {
    let layout = unsafe { GetKeyboardLayout(0) };
    println!("layout = {:?}", layout.0);
    for ch in ['a', 'V', 'x', 'Z', '7', ' ', '-'] {
        let unit = ch as u16;
        let r = unsafe { VkKeyScanExW(unit, layout) };
        if r == -1 {
            println!("{ch:?}: VkKeyScanExW = -1（此配置打不出來）");
            continue;
        }
        let vk = (r & 0xFF) as u16;
        let modifiers = (r >> 8) & 0xFF;
        let scan = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC) };
        println!("{ch:?}: raw={r:#06x} vk={vk:#04x} mods={modifiers:#04x} scan={scan:#x}");
    }
}
