searchState.loadedDescShard("kumo_dkim", 0, "DKIM errors\nBuilder for the Signer\nDKIM signature tag\nBuild an instance of the Signer Must be provided: …\nParse PKCS8 encoded ed25519 key data into a DkimPrivateKey.\nReturns the argument unchanged.\nReturns the argument unchanged.\nReturns the argument unchanged.\nReturns the argument unchanged.\nReturns the argument unchanged.\nReturns the argument unchanged.\nCalls <code>U::from(self)</code>.\nCalls <code>U::from(self)</code>.\nCalls <code>U::from(self)</code>.\nCalls <code>U::from(self)</code>.\nCalls <code>U::from(self)</code>.\nCalls <code>U::from(self)</code>.\nParse either an RSA or ed25519 key into a DkimPrivateKey. …\nName of the tag (v, i, a, h, …)\nNew builder\nMain entrypoint of the parser. Parses the DKIM signature …\nValue of the tag as seen in the text\nParse RSA key data into a DkimPrivateKey\nLoad RSA key data from a file and parse it into a …\nSign a message As specified in …\nValue of the tag with spaces removed\nRun the DKIM verification on the email\nRun the DKIM verification on the email providing an …\nSpecify the body canonicalization\nSpecify a expiry duration for the signature validity\nSpecify the header canonicalization\nEnable automatic over-signing. Each configured header in …\nSpecify the private key used to sign the email\nSpecify the private key used to sign the email\nSpecify headers to be used in the DKIM signature The From: …\nSpecify for which domain the email should be signed for\nSpecify current time. Mostly used for testing\nReturns the argument unchanged.\nCalls <code>U::from(self)</code>.")