// SPDX-License-Identifier: GPL-2.0

//! Hello Rust Kernel Module
//!
//! module!{} macro for metadata, 
//! kernel::Module 'Trait' is similar to the 'Interface' in JAVA and C# or the 'abstract class' in C++, 
//! and it is similar to struct of 'function pointer' in C
//! BUT you MUST implement the trait to complie in Rust.
//! Drop is similar to module_exit in C Kernal .

use kernel::prelude::*;

module! {
    type: HelloRust,
    name: "hello_rust",
    author: "Leeharim-korean",
    description: "Hello Rust kernel module for RPi 5",
    license: "GPL",
}

struct HelloRust;

impl kernel::Module for HelloRust {
    // When the module be loaded with 'insmod' command, the 'init' function calls.
    fn init(_module: &'static ThisModule) -> Result<Self> {
        // pr_info! == printk, and '!' means macro in Rust.
        pr_info!("Hello from Rust! (init)\n");
        pr_info!("Rust module loaded on Raspberry Pi 5\n");
        Ok(HelloRust)
    }
}

impl Drop for HelloRust {
    fn drop(&mut self) {
        pr_info!("Goodbye from Rust! (exit)\n");
    }
}
