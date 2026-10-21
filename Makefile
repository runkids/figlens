.PHONY: help devc devc-up devc-down devc-restart devc-reset devc-status

help:
	@echo "  make devc           # start the dev container and enter its shell"
	@echo "  make devc-up        # start the dev container (no shell)"
	@echo "  make devc-down      # stop the dev container"
	@echo "  make devc-restart   # restart the dev container"
	@echo "  make devc-reset     # remove the container and its cargo/target volumes"
	@echo "  make devc-status    # show the dev container state"

devc: devc-up
	docker compose exec dev bash

devc-up:
	docker compose up -d --build dev

devc-down:
	docker compose stop dev

devc-restart:
	docker compose restart dev

devc-reset:
	docker compose down -v

devc-status:
	docker compose ps dev
