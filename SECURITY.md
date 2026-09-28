# Security Policy

Security is important to KumoMTA. We welcome reports from security researchers, users, operators, and others who believe they have found a vulnerability.

## Reporting a Vulnerability

Please report suspected security vulnerabilities using [GitHub's private vulnerability reporting](https://github.com/KumoCorp/kumomta/security/advisories/new).

**Do not open a public GitHub issue for a suspected security vulnerability.**

A useful report will generally include:

- The affected KumoMTA version or commit.
- A description of the issue and its potential security impact.
- The conditions or configuration required to trigger the issue.
- Steps to reproduce it, ideally including a minimal reproducer.
- Any known mitigations or workarounds.
- Whether you are aware of active exploitation or public disclosure elsewhere.

You do not need to determine the severity or provide a CVE or CVSS score before reporting an issue. If you are unsure whether something has security implications, we would prefer that you report it privately and allow us to assess it.

Please avoid including sensitive information in public issues, pull requests, commit messages, or discussions while a vulnerability is being investigated.

## What Happens Next

We will review reports submitted through the private vulnerability reporting process.

For confirmed vulnerabilities, fixes are developed on the `main` branch and released from there. KumoMTA does not maintain older release branches or backport fixes to previous releases.

Where appropriate, we may coordinate disclosure with the reporter and other affected parties using GitHub Security Advisories.

We ask reporters to keep vulnerability details confidential while we investigate and prepare a fix.

If a report does not represent a security vulnerability, we may suggest that it be handled through the normal public issue tracker instead.

## Releases and Support

KumoMTA follows a forward-moving development and release model. Security fixes are made on `main` and become available in a subsequent release.

We do not provide security fixes or backports for older releases. Users should update to a current release to obtain security fixes.

Commercial support is provided to KumoMTA sponsors according to their applicable support arrangements. No particular response time, resolution time, or level of support is promised for other security reports.

Regardless of support status, we welcome responsible reports of suspected vulnerabilities through the private reporting process.

## Security Advisories

Confirmed vulnerabilities may be published as [GitHub Security Advisories](https://github.com/KumoCorp/kumomta/security/advisories).

Published advisories may identify affected releases and the release containing the fix where that information is applicable.

## Scope

Security issues can include, but are not limited to:

- Remote code execution or memory-safety vulnerabilities.
- Authentication or authorization bypasses.
- Exposure of message content, credentials, DKIM keys, or other sensitive information.
- Vulnerabilities that allow an untrusted remote party to crash or significantly disrupt a KumoMTA service.
- SMTP, HTTP, or other protocol handling issues with a meaningful security impact.
- Security boundary violations caused by crafted messages or untrusted input.

Ordinary bugs, configuration questions, performance issues, and crashes that cannot reasonably be triggered across a security boundary should normally be reported through the public GitHub issue tracker.

If you are uncertain whether an issue qualifies, report it privately.

Thank you for helping keep KumoMTA and its users secure.
