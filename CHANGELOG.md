# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.4] - 2022-09-14

### Changed

- Allow case insensitivity when searching for the correct signature header based on the domain of the From header.
## [0.2.3] - 2022-08-10

### Changed

- Apply canonicalization algorithms to DKIM header as well, to deal with linebreaks.
## [0.2.2] - 2022-08-09

### Changed

- Add support for ed25519
- Fix simple header canonicalization
## [0.2.1] - 2022-08-05

### Changed

- The successful dkim response now includes the canonicalization algorithms used for headers and body.
## [0.2.0] - 2022-07-20

### Changed

- This library no longer performs the decoding of the SMTP transparency encoding before generating a signature.

## [0.1.5] - 2022-06-28

### Added

- Exposed method in order to be able to provide an existing resolver.
