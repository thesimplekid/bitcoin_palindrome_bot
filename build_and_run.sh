#!/bin/bash

NAME=bitcoin_palindrom_bot

docker build -t $NAME . && \
    docker run --rm -ti --name $NAME $NAME
