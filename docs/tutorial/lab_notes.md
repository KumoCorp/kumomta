# Lab Notes

This page is a collection of interesing notes worth mentioning if you are experimenting with configs.

## TOML Files

TOML files can be super handy for abstracting config settings from the underlying Lua.  For instance, you might have 500 DKIM keys for 
domains you manage, but the DKIM signing process is always the same. You can create one signing configuration in Lua and have it read 
in the 500 sets of settings from a single TOML file.

Be aware that TOML files dont really tolerate duplicate entries. If you have duplicate keys unter a heading, unpredictable things can happen.

TOML files can be much easier to use than other forms of config files because due to the easy to use structure and the ability to comment.

IE:
[Top_Heading]
  "key" = "value"
# This is a comment.


