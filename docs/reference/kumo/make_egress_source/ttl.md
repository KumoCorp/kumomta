# ttl

Optional *time-to-live* specifying how long the source definition should be
cached.  The cache has two purposes:

* To limit the number of configurations kept in memory at any one time
* To enable data to be refreshed from external storage, such as a json data
  file, or a database

The default TTL is 60 seconds, but you can specify any duration using a string
like `"5 mins"` to specify 5 minutes.


