# Installing KumoMTA

##Using a Docker container for commercial mailing
Skip the instructions below  and just download a Docker conatiner from <link>.


##Installing for active development
Deploy a suitabls server (instance).  
So far this is tested on Rocky 8, ...

Note that in order for KumoMTA to bind to port 25 for outbound mail, it must be run as a privileged user.

```
sudo dnf install -y git
git clone https://github.com/wez/kumomta.git
cd kumomta
cargo run --release -p kumod -- --policy simple_policy.lua
```

In the above you are telling Cargo to run the Rust compiler to build an optimized release version and package it as kumod, then execure kumod using the policy file called simple_polict.lua.

If you are planning to just "use" KumoMTA and not develop against it, then you are better off using a Docker Image.  See above section for more on that. 

You can add debugging output by adding KUMOD_LOG=kumod=trace in the environment when you start kumod.

