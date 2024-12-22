# Darwin Push Port Live Feed

[![CI](https://github.com/viral32111/darwin-push-port-live-feed/actions/workflows/ci.yml/badge.svg)](https://github.com/viral32111/darwin-push-port-live-feed/actions/workflows/ci.yml)
[![Analyse](https://github.com/viral32111/darwin-push-port-live-feed/actions/workflows/analyse.yml/badge.svg)](https://github.com/viral32111/darwin-push-port-live-feed/actions/workflows/analyse.yml)
![GitHub tag (with filter)](https://img.shields.io/github/v/tag/viral32111/darwin-push-port-live-feed?label=Latest)
![GitHub repository size](https://img.shields.io/github/repo-size/viral32111/darwin-push-port-live-feed?label=Size)
![GitHub release downloads](https://img.shields.io/github/downloads/viral32111/darwin-push-port-live-feed/total?label=Downloads)
![GitHub commit activity](https://img.shields.io/github/commit-activity/m/viral32111/darwin-push-port-live-feed?label=Commits)

This project is a [STOMP client](https://stomp.github.io/stomp-specification-1.2.html) that maintains a local copy of real-time updates from the [National Rail Enquiries](https://www.nationalrail.co.uk/) [Darwin Push Port Live Feed](https://wiki.openraildata.com/index.php?title=Darwin:Push_Port#Darwin_Live_Feed_Topic) in a database.

This is designed to be minimal and lightweight so that it can be deployed as a microservice and scaled appropriately.

Performance is a key consideration as the [Darwin Push Port Live Feed](https://wiki.openraildata.com/index.php?title=Darwin:Push_Port#Darwin_Live_Feed_Topic) dispatches messages at a high rate, and this client must be able to process them in real-time.

This is my first Rust project, so I am learning as I go. I am open to feedback and suggestions.

I have been working on this project locally since the 3rd of March 2024, and only started tracking it on GitHub from the 21st of April 2024.

## ⚖️ License

Copyright (C) 2024 [viral32111](https://viral32111.com).

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see https://www.gnu.org/licenses.
