# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1] - 2022-08-05

### Changed

- The successful dkim response now includes the canonicalization algorithms used for headers and body.
## [0.2.0] - 2022-07-20

### Changed

- This library no longer performs the decoding of the SMTP transparency encoding before generating a signature.

## [0.1.5] - 2022-06-28

### Added

- Exposed method in order to be able to provide an existing resolver.
