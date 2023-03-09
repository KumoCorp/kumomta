# Installing KumoMTA in CentOS7

## Special case for development work in CentOS7

Note that Red Hat full support for RHEL 7 [ended in August 2019](https://access.redhat.com/support/policy/updates/errata#Retired_Life_Cycle_Dates) and CentOS 7 full support [ended in August 2020](https://wiki.centos.org/About/Product)

Also note that in testing, this process took several hours.

This first starts by adding dnf so all the rest of the install is consistent.

Next, You will need to install a few things in order to get this current.

```bash

# Get dnf installed first
sudo yum install -y dnf

# Now clean up, update and get the basics
sudo dnf clean all
sudo dnf update -y
sudo dnf group install -y "Development Tools"
sudo dnf install -y libxml2 libxml2-devel clang telnet git

# Now for the extra lifting we need to get CentOS7 to a relatively current state 
sudo dnf -y install bzip2 wget gcc gcc-c++ gmp-devel mpfr-devel libmpc-devel make openssl-devel
sudo dnf install -y centos-release-scl 
sudo dnf install -y llvm-toolset-7 devtoolset-9 devtoolset-9-gcc-c++ python3

# And now we need to make the compiler "current"
# Set us up in the right directory first
sudo -s
export PREFIX="/usr/share"
cd $PREFIX


# Get a newer version of GCC-C++ from source
# This part will take a while so maybe go get lunch... (About 40 minutes)
cd $PREFIX
wget https://ftp.gnu.org/gnu/gcc/gcc-12.2.0/gcc-12.2.0.tar.xz

tar xf gcc-12.2.0.tar.xz
mkdir gcc-12.2.0-build
cd gcc-12.2.0-build
../gcc-12.2.0/configure --enable-languages=c,c++ --disable-multilib --prefix=$PREFIX/gcc/12.2.0

make -j$(nproc)
make install

cd ..
rm -rf gcc-12.2.0 gcc-12.2.0-build gcc-12.2.0.tar.xz
echo "export CC=$PREFIX/gcc/12.2.0/bin/gcc" >> ~/.bashrc
source ~/.bashrc
echo "export CXX=$PREFIX/gcc/12.2.0/bin/g++" >> ~/.bashrc
source ~/.bashrc
echo "export FC=$PREFIX/gcc/12.2.0/bin/gfortran" >> ~/.bashrc
source ~/.bashrc
echo "export PATH=$PREFIX/gcc/12.2.0/bin:$PATH" >> ~/.bashrc
source ~/.bashrc
echo "export LD_LIBRARY_PATH=$PREFIX/gcc/12.2.0/lib64:$LD_LIRBARY_PATH" >> ~/.bashrc
source ~/.bashrc



# Get the latest version of cmake from source (About 20 minutes)
wget https://github.com/Kitware/CMake/releases/download/v3.25.3/cmake-3.25.3.tar.gz
tar zxf cmake-3.25.3.tar.gz
mv cmake-3.25.3.tar.gz /tmp/

cd cmake-3.25.3
./bootstrap && make && sudo make install
ln $PREFIX/cmake-3.25.3/bin/cmake /bin/cmake
mv $PREFIX/cmake-3.25.3 $PREFIX/cmake-3.25


# Get the latest version of llvm (clang) from source
cd $PREFIX
git clone --depth=1 https://github.com/llvm/llvm-project.git
cd llvm-project
cmake -S llvm -B build -G "Unix Makefiles" -DCMAKE_BUILD_TYPE=Release
```
