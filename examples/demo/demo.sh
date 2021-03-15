./target/debug/node_admin -t=test -i=Alice \
-s=./test-output/crypto-0 \
-b=./test-output/peer_addrs \
-d=mysql://root:helloworld@localhost:3306/hotstuff_test_Alice \
--tx-server-addr=127.0.0.1:12340


./target/debug/node_admin -t=test -i=Bob \
-s=./test-output/crypto-1 \
-b=./test-output/peer_addrs \
-d=mysql://root:helloworld@localhost:3306/hotstuff_test_Bob \
--tx-server-addr=127.0.0.1:12341


./target/debug/node_admin -t=test -i=Carol \
-s=./test-output/crypto-2 \
-b=./test-output/peer_addrs \
-d=mysql://root:helloworld@localhost:3306/hotstuff_test_Carol \
--tx-server-addr=127.0.0.1:12342


./target/debug/node_admin -t=test -i=Dave \
-s=./test-output/crypto-3 \
-b=./test-output/peer_addrs \
-d=mysql://root:helloworld@localhost:3306/hotstuff_test_Dave \
--tx-server-addr=127.0.0.1:12343