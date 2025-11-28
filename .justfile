# This section has all the testing commands. The first rule is the
# default and it tests every subcrate.

test-all: test-api _test-drivers test-drmemd
	@echo "All tests passed successfuly!"

test-api:
	@echo "Running units tests for drmem-api"; \
	nice cargo test -p drmem-api

_test-drivers: test-drv-ntp test-drv-sump test-drv-wu test-drv-tplink

test-drv-ntp: test-api
	@echo "Running units tests for drmem-driver-ntp"; \
	nice cargo test -p drmem-drv-ntp

test-drv-sump: test-api
	@echo "Running units tests for drmem-driver-sump"; \
	nice cargo test -p drmem-drv-sump

test-drv-tplink: test-api
	@echo "Running units tests for drmem-driver-tplink"; \
	nice cargo test -p drmem-drv-tplink

test-drv-wu: test-api
	@echo "Running units tests for drmem-driver-weather-wu"; \
	nice cargo test -p drmem-drv-weather-wu

_test-simple: test-api
	@echo "Running units tests for simple-backend, no-client"; \
	nice cargo test --features simple-backend,no-client

_test-simple-graphql: test-api
	@echo "Running units tests for simple-backend, with GraphQL"; \
	nice cargo test --features simple-backend,graphql

_test-redis-graphql: test-api
	@echo "Running units tests for redis-backend, with GraphQL"; \
	nice cargo test --features redis-backend,graphql

test-drmemd: _test-simple _test-simple-graphql _test-redis-graphql

# This section has the targets for checking the syntax and
# correctness. These commands run faster than the tests because they
# don't generate object files and it doesn't link anything together.

_check-simple:
	@echo "Checking simple-backend, no-client"; \
	nice cargo check --features simple-backend,no-client,all-drivers

_check-simple-graphql:
	@echo "Checking simple-backend, with GraphQL"; \
	nice cargo check --features simple-backend,graphql,all-drivers

_check-redis-graphql:
	@echo "Checking redis-backend, with GraphQL"; \
	nice cargo check --features redis-backend,graphql,all-drivers

check: _check-simple _check-simple-graphql _check-redis-graphql
	@echo "DrMem source was checked successfully!"

# This section helps publish the project to crates.io.

publish:
	@echo "DrMem Project published successfully!"

# Local variables:
# mode: makefile
# End:
