On a cache miss, have `Cache::get` fall back to loading the value from `/var/cache/<key>` on disk
(read the file at that path; treat a missing/unreadable file as "not found").
