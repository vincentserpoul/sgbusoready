# Security Policy

## Supported Versions

Only the latest commit on `main` is supported.

## Reporting a Vulnerability

If you discover a security vulnerability in SG Bus Ready, please report it responsibly:

1. **Do NOT** open a public GitHub issue for security vulnerabilities
2. Use GitHub's private vulnerability reporting: the **Report a vulnerability** button under the repository's **Security** tab
3. Include:
   - Description of the vulnerability
   - Steps to reproduce
   - Potential impact
   - Suggested fix (if any)

## Security Practices

This project follows these security practices:

### Dependency Management

- **cargo-audit**: Checks against the RustSec advisory database
- **cargo-deny**: Advisories, license compliance, banned crates and allowed sources
- **cargo-vet**: Supply-chain audit store in `.supply-chain/`
- **Dependabot**: Weekly grouped updates for cargo, GitHub Actions and Gradle, with a 7-day cooldown

### Code Quality

- **Clippy**: Strict linting, zero warnings in CI
- **`unsafe_code = "deny"`**: The Android JNI exports are the only exception, each with a stated reason
- **prek**: Pre-commit hooks, also enforced in CI

### Secret Management

- **Gitleaks**: Secret detection as a pre-commit hook and in CI
- No secrets stored in the repository
- The LTA `DataMall` `AccountKey` and the worker admin token are injected at runtime and never baked into the app or the container image

### CI/CD

- Security scanning on every PR and push to `main`, plus a weekly scheduled run
- GitHub Actions pinned to commit SHAs and linted with actionlint and zizmor
- Trivy filesystem and configuration scans

## Security Checklist for Contributors

- [ ] No hardcoded secrets, API keys, or credentials
- [ ] No `unsafe` blocks without a documented reason and review
- [ ] Dependencies are from crates.io
- [ ] New dependencies have a license on the `deny.toml` allowlist
- [ ] Input validation for all user-provided data
- [ ] Proper error handling (no `unwrap()` on untrusted input)
