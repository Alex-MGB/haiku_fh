This is a small prototype of Feed Handler which connects to the Deribit API (websocket), subscribe to some raw channels and then writes the data into a SHM.
Two SHM need to be created with HaikuSHM, 1 for the Orderbook, 1 for the Trades, they both have a specific layout. The offsets are provided by the component creating the SHM. You can refer to HaikuSHM for more details.


Right now the channels supported are:
- Raw orderbooks: 
  - We get a snapshot when we subscribe to the channels, then we only received update (new, change, delete)
  - The orderbooks are reconstructed, in practice we keep a bit more than 10 levels in case some delete happen, but we only write 10 levels.
  - The data are written into the appropriate SHM, respecting the layout.
- Raw trades:
  - The data are written into the Trade Ring Buffer

For these two channels a custom parser has be written to minimize the processing time. We try as much as possible to avoid any copy of data for the Trades and the OrderBook.
The other messages, such as Authentification, Subscription and Ping, are parsed through a slower parser.
The processing time (parsing + writing) takes in average around **1µs** depending of the size of the message to parse.

To write in the SHM we use the SHMAccessor provided by haiku_common.