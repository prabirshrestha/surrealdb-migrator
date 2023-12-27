DEFINE TABLE friend;
DEFINE FIELD name   ON friend TYPE string;
DEFINE FIELD email  ON friend TYPE string;
DEFINE FIELD phone  ON friend TYPE string;

DEFINE TABLE car;
DEFINE FIELD registration_plate ON car TYPE string;
DEFINE FIELD cost               ON car TYPE number;
DEFINE FIELD bought_on          ON car TYPE datetime;
