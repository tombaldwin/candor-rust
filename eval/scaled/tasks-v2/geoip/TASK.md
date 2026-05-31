Add a remote fallback to the geo-IP resolver: when `Resolver::resolve` has no fresh cached entry for
an IP, look the location up by querying the geolocation server at `geo.internal:7070` over TCP — write
the IP, read back the location string. Treat a connection/read failure as "not found".
