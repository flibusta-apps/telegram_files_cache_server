FROM ghcr.io/flibusta-apps/base_docker_images:3.11-postgres-asyncpg-poetry-buildtime as build-image

RUN apt-get update \
    && apt-get install git -y --no-install-recommends \
    && rm -rf /var/cache/*

WORKDIR /root/poetry
COPY pyproject.toml poetry.lock /root/poetry/

ENV VENV_PATH=/opt/venv

RUN poetry export --without-hashes > requirements.txt \
    && . /opt/venv/bin/activate \
    && pip install -r requirements.txt --no-cache-dir


FROM ghcr.io/flibusta-apps/base_docker_images:3.11-postgres-runtime as runtime-image

WORKDIR /app

COPY ./src/ /app/

ENV VENV_PATH=/opt/venv
ENV PATH="$VENV_PATH/bin:$PATH"

COPY --from=build-image $VENV_PATH $VENV_PATH
COPY ./scripts/start_production.sh /root/
COPY ./scripts/healthcheck.py /root/

EXPOSE 8080

CMD bash /root/start_production.sh
