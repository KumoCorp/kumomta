#!/bin/bash

bindgen unbound/libunbound/unbound.h -o src/lib.rs \
  --no-layout-tests \
  --raw-line "#![allow(non_snake_case)]" \
  --raw-line "#![allow(non_camel_case_types)]" \
  --raw-line "#![allow(non_upper_case_globals)]" \
  --raw-line "#![allow(clippy::unreadable_literal)]" \
  --raw-line "#![allow(clippy::upper_case_acronyms)]" \
  --raw-line "#![allow(rustdoc::broken_intra_doc_links)]" \
  --generate=functions,types,vars \
