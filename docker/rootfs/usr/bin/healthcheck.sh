#!/bin/sh

# s6-svstat exits 0 even for a down service; require it to be up for 2s so restart loops are unhealthy.
check() {
  set -- $(/package/admin/s6/command/s6-svstat -o up,updownfor "/run/s6-rc/servicedirs/$1")
  [ "$1" = true ] && [ "$2" -ge 2 ]
}

check hbbr || exit 1
check hbbs || exit 1
