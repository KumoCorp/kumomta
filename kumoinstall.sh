##########################################################
# KumoMTA quick installer script 
# This simply executes all the typical install commands in 
# a bash script for convenience. No magic, just efficiency :)
# - Tom Mairs - 26 Feb 2023
##########################################################

# Ensure you have built a suitable server or instance for this first.  
IE: An AWS t2.medium (2 cores, 4Gb RAM) with 50Gb Storage is about as small as you want to attempt.

# This has been tested on Rocky 8, 

sudo dnf clean all
sudo dnf update -y
sudo dnf group install -y "Development Tools"
sudo dnf install -y libxml2 libxml2-devel clang telnet

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source ~/.profile
source ~/.cargo/env
rustc -V

sudo dnf install -y git
git clone https://github.com/kumomta/kumomta.git

cd kumomta
KUMOD_LOG=kumod=trace cargo run -p kumod -- --policy simple_policy.lua

echo "We're done here - thanks for waiting :)"
