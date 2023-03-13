# Installing KumoMTA in CentOS7

Note that Red Hat full support for RHEL 7 [ended in August 2019](https://access.redhat.com/support/policy/updates/errata#Retired_Life_Cycle_Dates) and CentOS 7 full support [ended in August 2020](https://wiki.centos.org/About/Product). While KumoMTA is available for CentOS7, it is also available for almost any other Linux distro and we recommend upgrading to a newer OS as soon as possible.

...

First prepare your system by making sure it has the most current updates, includes wget, and any testing tools you need like telnet and curl.

To run KumoMTA in Centos7, download the prebuilt RPM and policy.

RPM: [https://github.com/kumomta/kumomta/suites/11445755838/artifacts/590348846](https://github.com/kumomta/kumomta/suites/11445755838/artifacts/590348846)
  
Simple policy: [https://github.com/kumomta/kumomta/blob/main/simple_policy.lua](https://github.com/kumomta/kumomta/blob/main/simple_policy.lua)
  
Sink policy: [https://github.com/kumomta/kumomta/blob/main/sink.lua](https://github.com/kumomta/kumomta/blob/main/sink.lua)
  
You should `unzip centos7.zip`
  
Then install with `rpm -ivh centos7/kumomta-2023.03.08_b3fa0dab-1.centos7.x86_64`
  
This will install a working copy of KumoMTA at `/usr/bin/kumod`
 
You can pull a copy of the simple_policy.lua or sink.lua and then run it like:

`/usr/bin/kumod --policy simple_policy.lua`
  
  **OR**
  
Follow this to do it from the command line:

```bash
# Prepare the system first
sudo yum install -y dnf
sudo dnf clean all
sudo dnf update -y
sudo dnf install -y libxml2 libxml2-devel clang curl telnet git bzip2 wget openssl-devel

# Now install KumoMTA
cd
sudo wget https://github.com/kumomta/kumomta/suites/11445755838/artifacts/590348846
sudo wget https://github.com/kumomta/kumomta/blob/main/simple_policy.lua
sudo wget https://github.com/kumomta/kumomta/blob/main/sink.lua
sudo unzip centos7.zip
rpm -ivh centos7/kumomta-2023.03.08_b3fa0dab-1.centos7.x86_64.rpm
sudo /usr/bin/kumod --policy sink.lua --user $USER
```

You should now be running KumoMTA in CentOS7

