"""empty message

Revision ID: f77b0b14f9eb
Revises: 9b7cfb422191
Create Date: 2022-12-30 22:53:41.951490

"""
from alembic import op
import sqlalchemy as sa


# revision identifiers, used by Alembic.
revision = "f77b0b14f9eb"
down_revision = "9b7cfb422191"
branch_labels = None
depends_on = None


def upgrade():
    op.add_column(
        "cached_files", sa.Column("message_id", sa.BigInteger(), nullable=True)
    )
    op.add_column("cached_files", sa.Column("chat_id", sa.BigInteger(), nullable=True))


def downgrade():
    op.drop_column("cached_files", "message_id")
    op.drop_column("cached_files", "chat_id")
