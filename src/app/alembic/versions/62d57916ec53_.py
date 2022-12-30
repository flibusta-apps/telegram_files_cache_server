"""empty message

Revision ID: 62d57916ec53
Revises: f77b0b14f9eb
Create Date: 2022-12-30 23:30:50.867163

"""
from alembic import op
import sqlalchemy as sa


# revision identifiers, used by Alembic.
revision = "62d57916ec53"
down_revision = "f77b0b14f9eb"
branch_labels = None
depends_on = None


def upgrade():
    op.drop_column("cached_files", "data")
    op.create_unique_constraint(
        "uc_cached_files_message_id_chat_id", "cached_files", ["message_id", "chat_id"]
    )
    op.create_index(
        op.f("ix_cached_files_message_id"), "cached_files", ["message_id"], unique=True
    )


def downgrade():
    op.add_column("cached_files", sa.Column("data", sa.JSON(), nullable=False))
    op.drop_constraint("uc_cached_files_message_id_chat_id", "cached_files")
    op.drop_index("ix_cached_files_message_id", "cached_files")
