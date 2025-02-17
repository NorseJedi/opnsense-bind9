# OPNSense Bind9 Host-sync
This is a handy little utility I wrote when playing around with Bind9 as a DNS server for my local network. All it does is download the DHCP leases from OPNSense and generates Bind9 zone-files with those hosts, both A-records and PTR-records.

## Building
You'll need to have [rust](https://www.rust-lang.org/) installed.
Simply run `cargo build -r` and the binary will be in `target/release/opnsense-bind9`

To build for another architecture, like say a Raspberry Pi, you can use [cross](https://github.com/cross-rs/cross). Install it with `cargo install cross`, then build with:
```
cross build --release --target=aarch64-unknown-linux-gnu --features reqwest/native-tls-vendored
```
The binary will then be found in `target/aarch64-unknown-linux-gnu/release/opnsense-bind9`

## Installation
Put the binary somewhere like /usr/local/sbin and put the config-file anywhere, the program will look for it in the following locations in order
* The same directory as the binary is in (`./opnsense-bind9.conf`)
* As a dot-file in the home directory of the user running the program (`~/.opnsense-bind9.conf`)
* `/usr/local/etc/opnsense-bind9.conf`
* `/etc/opnsense-bind9.conf`

The program can also be run with the `--conf` option followed by the path to the config-file:
```
opnsense-bind9 --conf /usr/local/etc/opnsense-bind9.conf
```

Run the program through cron. How often you run it is up to you.

## Bind9 config setup
Since this program overwrites the files with the hosts, you should just include them in your actual config-files. I recommend a setup like this for an example network using the domain `mydmain.example.com` and the network `192.168.0.0/24`.

**Note** This is just an example for illustration purposes, the important thing is that `hosts.conf` and `dhcphosts.conf` are included in the main zone configuration, and that `ptr-hosts.conf` is included in the reverse zone configuration. All file names and locations are configured in the config-file (except the `db.x.x.x.in-addr.arpa` file, which is always in the base config directory and named based on the IP addresses you set for the network). That being said, this is the type of setup I use, and it works nicely for me.

#### File/Directory structure
```
/
└── etc
    └── bind
        ├── db.0.168.192.in-addr.arpa
        ├── mydomain.example.com
        │   ├── dhcphosts.conf      # The generated file with A records
        │   ├── hosts.conf          # A file with static records
        │   ├── main.conf           # The main zone file
        │   └── ptr-dhcphosts.conf  # The generated file with PTR records 
        ├── named.conf
        ├── named.conf.local        # Your modified config-file
        ├── named.conf.options
        └── rndc.key
```
The files `named.conf`, `named.conf.options` and `rndc.key` are not covered here, but in a default Bind9 setup, `named.conf` will have an include statement for `named.conf.local` and that's all that's needed with regards to this.

The `hosts.conf` file here is included in `main.conf` and is just a file where you put any DNS records not handled by the OPNSense dhcp server. You could also just put them in the `main.conf` file, I just like to keep things separate. It's important however that any hosts you do define statically are put in `IGNORED_HOSTS` in the config-file, otherwise you'll get duplicates and Bind will yell at you.

#### Contents of `named.conf.local`:
```
include "/etc/bind/rndc.key";

controls {
    inet 127.0.0.1 port 953
    allow { 127.0.0.1; } keys { "rndc-key"; };
};

zone "mydomain.example.com" IN {
    type master;
    file "/etc/bind/mydomain.example.com/main.conf";
};

zone "0.168.192.in-addr.arpa" {
    type master;
    file "/etc/bind/db.0.168.192.in-addr.arpa";
};

```

#### Contents of `db.0.168.192.in-addr.arpa`:
```
$TTL 2d

@   IN  SOA ns1.mydomain.example.com. hostmaster.mydomain.example.com (
    2025010101  ; serial
    12h         ; refresh
    15m         ; retry
    3w          ; expire
    2h          ; min TTL
)
    IN  NS  ns1.mydomain.example.com.
    IN  NS  ns2.mydomain.example.com.

1   IN  PTR opnsense.mydomain.example.com.

$INCLUDE "/etc/bind/mydomain.example.com/ptr-dhcphosts.conf";
```

#### Contents of `mydomain.example.com/main.conf`:
```
$TTL 2d

$ORIGIN mydomain.example.com.

@   IN  SOA ns1.mydomain.example.com. hostmaster.mydomain.example.com. (
    2025010101  ; serial
    12h         ; refresh
    15m         ; retry
    3w          ; expire
    2h          ; min TTL
)

                        IN      NS      ns1.mydomain.example.com.
                        IN      NS      ns2.mydomain.example.com.
mydomain.example.com    IN      MX  10  mail.mydomain.example.com.

; Nameservers
ns1             IN      A       192.168.0.53
ns2             IN      A       192.168.0.54
pihole          IN      A       192.168.0.55

; Other hosts
$INCLUDE "/etc/bind/mydomain.example.com/hosts.conf";
$INCLUDE "/etc/bind/mydomain.example.com/dhcphosts.conf"
```
